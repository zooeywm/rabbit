use gpui::{
    AppContext, Context, IntoElement, ParentElement as _, Render, Window, WindowOptions, div,
};
use gpui_component::Root;

use crate::app::AppHandle;

struct TestUi {
    _app_handle: AppHandle,
}

impl Render for TestUi {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child("Rabbit Test UI")
    }
}

pub(crate) fn run(app_handle: AppHandle) -> eros::Result<()> {
    gpui_platform::application().run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| TestUi {
                    _app_handle: app_handle,
                });

                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open test UI window");
        })
        .detach();
    });
    Ok(())
}
