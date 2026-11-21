use leptos::wasm_bindgen::JsCast;

fn main() {
    console_error_panic_hook::set_once();
    if let Some(document) = web_sys::window().and_then(|window| window.document()) {
        let root = document
            .create_element("div")
            .expect("Storefront root")
            .unchecked_into::<web_sys::HtmlElement>();
        root.set_id("client-content");
        let has_server_content = document.get_element_by_id("server-content").is_some();
        root.set_hidden(has_server_content);
        document
            .body()
            .expect("Document body")
            .append_child(&root)
            .expect("Mount storefront root");
        leptos::mount::mount_to(root, brunch::app::App).forget();
        if !has_server_content && let Some(loading) = document.get_element_by_id("app-loading") {
            loading.remove();
        }
    }
}
