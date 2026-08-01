pub mod stream;

pub trait AggregateRoot {
    type Id: Copy;

    fn id(&self) -> Self::Id;
}
