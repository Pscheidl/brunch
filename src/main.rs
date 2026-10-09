fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(brunch::app::App);
    if let Some(document) = web_sys::window().and_then(|window| window.document()) {
        if let Some(loading) = document.get_element_by_id("app-loading") {
            loading.remove();
        }
    }
}
