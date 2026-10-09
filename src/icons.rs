use leptos::prelude::*;

#[component]
pub fn Icon(#[prop(default = "arrow")] name: &'static str) -> impl IntoView {
    let path = match name {
        "bag" => "M5 7h14l1 14H4L5 7Zm3 0V5a4 4 0 0 1 8 0v2",
        "pin" => "M20 10c0 6-8 12-8 12S4 16 4 10a8 8 0 1 1 16 0ZM15 10a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
        "clock" => "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0ZM12 7v5l3 2",
        "leaf" => "M20 3C8 2 2 7 5 15c7 5 16-1 15-12ZM3 21 15 9",
        "truck" => {
            "M2 5h12v12H2V5Zm12 5h4l4 4v3h-8M8 18a2 2 0 1 1-4 0 2 2 0 0 1 4 0Zm12 0a2 2 0 1 1-4 0 2 2 0 0 1 4 0Z"
        }
        "check" => "m5 12 4 4L19 6",
        "plus" => "M12 5v14M5 12h14",
        "minus" => "M5 12h14",
        "close" => "m6 6 12 12M6 18 18 6",
        "chevron" => "m8 10 4 4 4-4",
        "back" => "M19 12H5m6-6-6 6 6 6",
        "card" => "M3 5h18v14H3V5Zm0 5h18M6 15h4",
        "wallet" => "M3 5h15v3H3V5Zm0 3h18v12H3V8Zm13 5h5v4h-5v-4Z",
        "person" => "M16 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0ZM4 21v-2a8 8 0 0 1 16 0v2",
        "sparkle" => "m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5L12 3Z",
        "info" => "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0ZM12 11v6m0-10v.1",
        "store" => {
            "M4 10v11h16V10M2 10l2-7h16l2 7M2 10a3.3 3.3 0 0 0 6.6 0 3.4 3.4 0 0 0 6.8 0 3.3 3.3 0 0 0 6.6 0M9 21v-7h6v7"
        }
        "sun" => {
            "M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0ZM12 2v2m0 16v2M2 12h2m16 0h2M5 5l1 1m12 12 1 1M5 19l1-1M18 6l1-1"
        }
        _ => "M5 12h14m-6-6 6 6-6 6",
    };
    view! { <svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d=path/></svg> }
}
