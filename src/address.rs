use crate::api;
use leptos::{prelude::*, task::spawn_local};
use serde::Deserialize;
use std::time::Duration;

const HINT: &str = "Ulici a obec vyberte z návrhů RÚIAN nebo adresu napište ručně.";

#[derive(Clone, Deserialize)]
struct Address {
    label: String,
    street: String,
    city: String,
    postal_code: String,
}

#[component]
pub fn StreetAddress(
    street: RwSignal<String>,
    city: RwSignal<String>,
    postal: RwSignal<String>,
) -> impl IntoView {
    let matches = RwSignal::new(Vec::<Address>::new());
    let highlighted = RwSignal::new(None::<usize>);
    let revision = RwSignal::new(0u64);
    let hint = RwSignal::new(HINT);
    let dismiss = move || {
        revision.update(|id| *id += 1);
        matches.set(vec![]);
        highlighted.set(None);
        hint.set(HINT);
    };
    let select = move |index: usize| {
        if let Some(address) = matches.with_untracked(|rows| rows.get(index).cloned()) {
            street.set(address.street);
            city.set(address.city);
            postal.set(address.postal_code);
        }
        dismiss();
    };
    let on_input = move |ev| {
        let value = event_target_value(&ev);
        street.set(value.clone());
        dismiss();
        let text = value.trim().to_owned();
        if !(3..=100).contains(&text.chars().count()) {
            return;
        }
        // A slow reply must not replace a newer search or reopen a dismissed list.
        let current = revision.get_untracked();
        set_timeout(
            move || {
                if revision.try_get_untracked() != Some(current) {
                    return;
                }
                spawn_local(async move {
                    let result = api::post::<Vec<Address>>(
                        "/api/address-suggestions",
                        &serde_json::json!({ "text": text }),
                    )
                    .await;
                    if revision.try_get_untracked() == Some(current) {
                        match result {
                            Ok(rows) if !rows.is_empty() => matches.set(rows),
                            Ok(_) => hint.set("Žádná odpovídající adresa. Doplňte název obce nebo adresu napište ručně."),
                            Err(_) => hint.set("Našeptávač je dočasně nedostupný. Adresu můžete zadat ručně."),
                        }
                    }
                });
            },
            Duration::from_millis(350),
        );
    };
    let on_key = move |ev: leptos::ev::KeyboardEvent| {
        let count = matches.with_untracked(Vec::len);
        let active = highlighted.get_untracked();
        if count == 0 {
            return;
        }
        match (ev.key().as_str(), active) {
            ("ArrowDown", _) => highlighted.set(Some(active.map_or(0, |i| (i + 1).min(count - 1)))),
            ("ArrowUp", _) => {
                highlighted.set(Some(active.map_or(count - 1, |i| i.saturating_sub(1))))
            }
            ("Enter", Some(i)) => select(i),
            ("Escape", _) => dismiss(),
            _ => return,
        }
        ev.prevent_default();
        ev.stop_propagation();
    };

    view! {
        <div class="street-lookup">
            <label class="form-field">"Ulice a číslo domu"
                <input name="street-address" autocomplete="street-address" role="combobox"
                    aria-autocomplete="list" aria-controls="address-results"
                    aria-expanded=move || (!matches.with(Vec::is_empty)).to_string()
                    aria-activedescendant=move || highlighted.get().map(|i| format!("address-result-{i}"))
                    aria-describedby="address-hint" required pattern=r".*\S.*" maxlength="250"
                    prop:value=move || street.get() on:input=on_input on:keydown=on_key
                    on:blur=move |_| dismiss()/>
            </label>
            <ul id="address-results" role="listbox" aria-label="Návrhy adres"
                hidden=move || matches.with(Vec::is_empty)>
                {move || matches.get().into_iter().enumerate().map(|(i, address)| view! {
                    <li id=format!("address-result-{i}") role="option"
                        aria-selected=move || (highlighted.get() == Some(i)).to_string()
                        on:mousedown=|ev| ev.prevent_default() on:click=move |_| select(i)>
                        {address.label}
                    </li>
                }).collect_view()}
            </ul>
            <p class="field-hint" id="address-hint" role="status">{move || hint.get()}</p>
        </div>
    }
}
