use std::{
    collections::VecDeque,
    fs::File,
    io,
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
};

use eros::Context;
use flume::{Receiver, Sender, unbounded};

use drm::buffer::{DrmFourcc, DrmModifier};

use crate::kernel::geometry::PixelSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DmaBufProfile {
    pub(crate) format: DrmFourcc,
    pub(crate) modifier: DrmModifier,
}

#[derive(Debug)]
pub(crate) struct DmaBufObject {
    pub(crate) fd: OwnedFd,
    pub(crate) size: usize,
}

impl TryFrom<OwnedFd> for DmaBufObject {
    type Error = io::Error;

    fn try_from(fd: OwnedFd) -> Result<Self, Self::Error> {
        let file = File::from(fd);
        let size = usize::try_from(file.metadata()?.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "DMA-BUF object length exceeds usize",
            )
        })?;

        if size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DMA-BUF object has zero length",
            ));
        }

        Ok(Self {
            fd: OwnedFd::from(file),
            size,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DmaBufPlane {
    pub(crate) object_index: usize,
    pub(crate) offset: u32,
    pub(crate) stride: u32,
    pub(crate) modifier: DrmModifier,
}

#[derive(Debug)]
pub(crate) struct DmaBufFrame {
    pub(crate) size: PixelSize,
    pub(crate) format: DrmFourcc,
    pub(crate) objects: Vec<DmaBufObject>,
    pub(crate) planes: Vec<DmaBufPlane>,
    pub(crate) readiness_fence: Option<OwnedFd>,
    pub(crate) lease: Option<DmaBufLease>,
    pub(crate) va_backing: Option<DmaBufVaBacking>,
}

#[derive(Debug, Clone)]
pub(crate) struct DmaBufVaBacking {
    pub(crate) buffer: gstreamer::Buffer,
    pub(crate) context: gstreamer::Context,
}

#[derive(Debug)]
pub(crate) struct DmaBufRelease {
    pub(crate) slot: usize,
    pub(crate) readiness_fence: Option<OwnedFd>,
    pub(crate) reusable: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DmaBufLease(Arc<DmaBufLeaseInner>);

#[derive(Debug)]
struct DmaBufLeaseInner {
    slot: usize,
    release: Sender<DmaBufRelease>,
    state: Mutex<DmaBufLeaseState>,
}

#[derive(Debug, Default)]
struct DmaBufLeaseState {
    readiness_fence: Option<OwnedFd>,
    reusable: bool,
}

impl DmaBufFrame {
    pub(crate) fn try_clone(&self) -> eros::Result<Self> {
        let mut objects = Vec::with_capacity(self.objects.len());

        for (object_index, object) in self.objects.iter().enumerate() {
            let fd = object.fd.try_clone().with_context(|| {
                format!("Failed to duplicate cached DMA-BUF object {object_index}")
            })?;
            objects.push(DmaBufObject {
                fd,
                size: object.size,
            });
        }

        Ok(Self {
            size: self.size,
            format: self.format,
            objects,
            planes: self.planes.clone(),
            readiness_fence: None,
            lease: None,
            va_backing: self.va_backing.clone(),
        })
    }

    pub(crate) fn try_clone_with_lease(&self, lease: DmaBufLease) -> eros::Result<Self> {
        let mut objects = Vec::with_capacity(self.objects.len());

        for (object_index, object) in self.objects.iter().enumerate() {
            let fd = object.fd.try_clone().with_context(|| {
                format!("Failed to duplicate pooled DMA-BUF object {object_index}")
            })?;
            objects.push(DmaBufObject {
                fd,
                size: object.size,
            });
        }

        Ok(Self {
            size: self.size,
            format: self.format,
            objects,
            planes: self.planes.clone(),
            readiness_fence: None,
            lease: Some(lease),
            va_backing: self.va_backing.clone(),
        })
    }

    pub(crate) fn set_release_fence(&self, fence: OwnedFd) {
        if let Some(lease) = &self.lease {
            lease.set_release_fence(fence);
        }
    }

    pub(crate) fn invalidate_lease(&self) {
        if let Some(lease) = &self.lease {
            lease.invalidate();
        }
    }
}

impl DmaBufLease {
    pub(crate) fn new(slot: usize, release: Sender<DmaBufRelease>) -> Self {
        Self(Arc::new(DmaBufLeaseInner {
            slot,
            release,
            state: Mutex::new(DmaBufLeaseState {
                readiness_fence: None,
                reusable: true,
            }),
        }))
    }

    pub(crate) fn set_release_fence(&self, fence: OwnedFd) {
        let mut state = match self.0.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.readiness_fence = Some(fence);
    }

    pub(crate) fn invalidate(&self) {
        let mut state = match self.0.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.reusable = false;
        state.readiness_fence = None;
    }
}

impl Drop for DmaBufLeaseInner {
    fn drop(&mut self) {
        let state = match self.state.get_mut() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let _ = self.release.send(DmaBufRelease {
            slot: self.slot,
            readiness_fence: state.readiness_fence.take(),
            reusable: state.reusable,
        });
    }
}

#[derive(Debug)]
pub(crate) struct DmaBufPool {
    capacity: usize,
    slots: Vec<Option<DmaBufFrame>>,
    available: VecDeque<usize>,
    release: Sender<DmaBufRelease>,
    released: Receiver<DmaBufRelease>,
}

impl DmaBufPool {
    pub(crate) fn new(capacity: usize) -> Self {
        let (release, released) = unbounded();

        Self {
            capacity,
            slots: Vec::with_capacity(capacity),
            available: VecDeque::with_capacity(capacity),
            release,
            released,
        }
    }

    pub(crate) fn acquire(
        &mut self,
        mut allocate: impl FnMut() -> eros::Result<DmaBufFrame>,
        mut wait_on_fence: impl FnMut(OwnedFd) -> eros::Result<()>,
    ) -> eros::Result<Option<DmaBufFrame>> {
        while let Ok(release) = self.released.try_recv() {
            if !release.reusable {
                self.slots[release.slot] = None;
                continue;
            }
            if let Some(fence) = release.readiness_fence
                && let Err(error) = wait_on_fence(fence)
            {
                self.slots[release.slot] = None;
                return Err(error);
            }
            self.available.push_back(release.slot);
        }

        let slot = match self.available.pop_front() {
            Some(slot) => slot,
            None => {
                let active = self.slots.iter().filter(|slot| slot.is_some()).count();
                if active == self.capacity {
                    return Ok(None);
                }
                let frame = allocate()?;
                match self.slots.iter().position(Option::is_none) {
                    Some(slot) => {
                        self.slots[slot] = Some(frame);
                        slot
                    }
                    None => {
                        self.slots.push(Some(frame));
                        self.slots.len() - 1
                    }
                }
            }
        };
        let frame = self.slots[slot]
            .as_ref()
            .with_context(|| "DMA-BUF pool selected an empty slot")?;
        let lease = DmaBufLease::new(slot, self.release.clone());

        Ok(Some(frame.try_clone_with_lease(lease)?))
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs::File, os::fd::OwnedFd};

    use drm::buffer::DrmFourcc;

    use crate::{
        infra::platform::dma_buf::{DmaBufFrame, DmaBufObject, DmaBufPool},
        kernel::geometry::PixelSize,
    };

    #[test]
    fn pool_reuses_only_after_every_lease_holder_releases() {
        let allocations = Cell::new(0);
        let fence_waits = Cell::new(0);
        let mut pool = DmaBufPool::new(1);
        let mut allocate = || {
            allocations.set(allocations.get() + 1);
            Ok(test_frame())
        };
        let first = pool
            .acquire(&mut allocate, |_| Ok(()))
            .expect("First pool acquisition should succeed")
            .expect("First pool acquisition should allocate a frame");
        let retained = first
            .lease
            .as_ref()
            .expect("Pooled frame should carry a lease")
            .clone();
        first.set_release_fence(OwnedFd::from(
            File::open("/dev/zero").expect("Test release fence placeholder should open"),
        ));

        drop(first);
        assert!(
            pool.acquire(&mut allocate, |_| {
                fence_waits.set(fence_waits.get() + 1);
                Ok(())
            })
            .expect("Busy pool acquisition should succeed")
            .is_none()
        );

        drop(retained);
        let reused = pool
            .acquire(&mut allocate, |_| {
                fence_waits.set(fence_waits.get() + 1);
                Ok(())
            })
            .expect("Released pool acquisition should succeed")
            .expect("Released slot should be reused");

        assert_eq!(allocations.get(), 1);
        assert_eq!(fence_waits.get(), 1);
        drop(reused);
    }

    #[test]
    fn invalidated_pool_slot_is_replaced() {
        let allocations = Cell::new(0);
        let mut pool = DmaBufPool::new(1);
        let mut allocate = || {
            allocations.set(allocations.get() + 1);
            Ok(test_frame())
        };
        let frame = pool
            .acquire(&mut allocate, |_| Ok(()))
            .expect("First pool acquisition should succeed")
            .expect("First pool acquisition should allocate a frame");
        frame.invalidate_lease();
        drop(frame);

        let replacement = pool
            .acquire(&mut allocate, |_| Ok(()))
            .expect("Replacement pool acquisition should succeed")
            .expect("Invalidated slot should be replaced");

        assert_eq!(allocations.get(), 2);
        drop(replacement);
    }

    fn test_frame() -> DmaBufFrame {
        DmaBufFrame {
            size: PixelSize {
                width: 16,
                height: 16,
            },
            format: DrmFourcc::Xrgb8888,
            objects: vec![DmaBufObject {
                fd: OwnedFd::from(
                    File::open("/dev/zero").expect("Test DMA-BUF placeholder should open"),
                ),
                size: 1,
            }],
            planes: Vec::new(),
            readiness_fence: None,
            lease: None,
            va_backing: None,
        }
    }
}
