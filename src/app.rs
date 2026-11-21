use crate::{
    api,
    catalog::{
        self, Cart, DeliveryOptions, Order, OrderRequest, PaymentRecovery, PaymentResponse,
        Product, RequestedItem, Settings, Terms, User, UserResponse, money,
    },
    delivery_calendar::{
        DeliveryCalendar, date_label, earliest_date, prague_today, selectable_date,
    },
    icons::Icon,
};
use leptos::{html, prelude::*, task::spawn_local};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[component]
pub fn App() -> impl IntoView {
    view! { <Home/> }
}

#[component]
fn Brand(#[prop(default = "brand")] class: &'static str) -> impl IntoView {
    view! {
        <a class=class href="./" aria-label="Brunch Era, úvodní stránka">
            <img class="brand-wordmark" src="brand/wordmark.svg" width="704" height="168" alt=""/>
        </a>
    }
}

#[derive(Clone, Copy)]
struct Details {
    name: RwSignal<String>,
    email: RwSignal<String>,
    phone: RwSignal<String>,
    organization: RwSignal<String>,
    street: RwSignal<String>,
    city: RwSignal<String>,
    postal: RwSignal<String>,
    notes: RwSignal<String>,
    billing_enabled: RwSignal<bool>,
    billing_name: RwSignal<String>,
    billing_address: RwSignal<String>,
    billing_company_id: RwSignal<String>,
    billing_tax_id: RwSignal<String>,
}
impl Details {
    fn new() -> Self {
        Self {
            name: RwSignal::new(String::new()),
            email: RwSignal::new(String::new()),
            phone: RwSignal::new(String::new()),
            organization: RwSignal::new(String::new()),
            street: RwSignal::new(String::new()),
            city: RwSignal::new(String::new()),
            postal: RwSignal::new(String::new()),
            notes: RwSignal::new(String::new()),
            billing_enabled: RwSignal::new(false),
            billing_name: RwSignal::new(String::new()),
            billing_address: RwSignal::new(String::new()),
            billing_company_id: RwSignal::new(String::new()),
            billing_tax_id: RwSignal::new(String::new()),
        }
    }
    fn use_address(self, order: &Order) {
        self.name.set(order.name.clone().unwrap_or_default());
        self.phone.set(order.phone.clone().unwrap_or_default());
        self.organization
            .set(order.organization.clone().unwrap_or_default());
        self.notes.set(order.notes.clone().unwrap_or_default());
        if order.is_pickup() {
            return;
        }
        self.street.set(order.street.clone());
        self.city.set(order.city.clone());
        self.postal
            .set(catalog::normalize_postal_code(&order.postal_code));
        self.notes.set(order.notes.clone().unwrap_or_default());
    }
}
#[derive(Clone, Copy)]
struct Shop {
    pickup: RwSignal<bool>,
    pickup_selection: RwSignal<Option<catalog::PickupSettings>>,
    cart: RwSignal<Cart>,
    products: RwSignal<Vec<Product>>,
    settings: RwSignal<Settings>,
    options: RwSignal<DeliveryOptions>,
    terms: RwSignal<Option<Terms>>,
    accepted_terms: RwSignal<Option<i64>>,
    terms_loading: RwSignal<bool>,
    terms_error: RwSignal<String>,
    loading: RwSignal<bool>,
    ready: RwSignal<bool>,
    error: RwSignal<String>,
    notice: RwSignal<String>,
    announcement: RwSignal<String>,
    cart_dialog: NodeRef<html::Dialog>,
    delivery_dialog: NodeRef<html::Dialog>,
    product_dialog: NodeRef<html::Dialog>,
    account_dialog: NodeRef<html::Dialog>,
    selected_product: RwSignal<i64>,
    favorites: RwSignal<BTreeSet<i64>>,
    variants: RwSignal<BTreeMap<i64, String>>,
    team_order_id: RwSignal<Option<String>>,
    team_revision: RwSignal<Option<i64>>,
    auth_sent: RwSignal<bool>,
    auth_countdown: RwSignal<u16>,
    day: RwSignal<String>,
    today: RwSignal<Option<chrono::NaiveDate>>,
    slot: RwSignal<String>,
    step: RwSignal<u8>,
    cart_title: NodeRef<html::H2>,
    cart_content: NodeRef<html::Div>,
    details: Details,
    payment: RwSignal<String>,
    submitting: RwSignal<bool>,
    order_error: RwSignal<String>,
    confirmation: RwSignal<Option<Order>>,
    user: RwSignal<Option<User>>,
    orders: RwSignal<Vec<Order>>,
    account_loading: RwSignal<bool>,
    account_error: RwSignal<String>,
    auth_message: RwSignal<String>,
    auth_busy: RwSignal<bool>,
    login_token: RwSignal<Option<String>>,
    attempt: RwSignal<Option<(String, String)>>,
    payment_recovery: RwSignal<Option<PaymentRecovery>>,
    payment_busy: RwSignal<bool>,
    payment_state: RwSignal<String>,
    payment_error: RwSignal<String>,
}
fn open_dialog(dialog: NodeRef<html::Dialog>) {
    if let Some(d) = dialog.get() {
        let _ = d.show_modal();
    }
}
fn close_dialog(dialog: NodeRef<html::Dialog>) {
    if let Some(d) = dialog.get() {
        d.close();
    }
}
fn update_postal_input(event: &leptos::ev::Event, postal: RwSignal<String>) {
    let input = event_target::<web_sys::HtmlInputElement>(event);
    let value = input.value();
    let normalized = catalog::normalize_postal_code(&value);
    if normalized != value {
        input.set_value(&normalized);
    }
    postal.set(normalized);
}
fn storage_get(key: &str, session: bool) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window()?;
        let storage = if session {
            window.session_storage()
        } else {
            window.local_storage()
        }
        .ok()??;
        storage.get_item(key).ok().flatten()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (key, session);
        None
    }
}
fn stored_order_attempt() -> Option<(String, String)> {
    storage_get("brunch-order-attempt", true)
        .and_then(|value| serde_json::from_str::<(String, String)>(&value).ok())
        .filter(|(signature, key)| {
            !key.is_empty()
                && serde_json::from_str::<serde_json::Value>(signature)
                    .is_ok_and(|value| value.is_object())
        })
}
fn storage_set(key: &str, value: Option<&str>, session: bool) {
    #[cfg(target_arch = "wasm32")]
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = if session {
            window.session_storage()
        } else {
            window.local_storage()
        } {
            if let Some(value) = value {
                let _ = storage.set_item(key, value);
            } else {
                let _ = storage.remove_item(key);
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (key, value, session);
}
fn new_key() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()?.crypto().ok().map(|c| c.random_uuid())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}
fn payment_deadline(value: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let options = js_sys::Object::new();
        for (key, value) in [
            ("dateStyle", "medium"),
            ("timeStyle", "short"),
            ("timeZone", "Europe/Prague"),
        ] {
            let _ = js_sys::Reflect::set(&options, &key.into(), &value.into());
        }
        js_sys::Date::new(&value.into())
            .to_locale_string("cs-CZ", &options)
            .into()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        value.to_owned()
    }
}
fn take_login_token() -> Option<String> {
    take_query_parameter("login_token")
}
fn take_query_parameter(name: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window()?;
        let url = web_sys::Url::new(&window.location().href().ok()?).ok()?;
        let token = url.search_params().get(name)?;
        url.search_params().delete(name);
        let _ = window.history().ok()?.replace_state_with_url(
            &js_sys::Object::new().into(),
            "",
            Some(&format!("{}{}{}", url.pathname(), url.search(), url.hash())),
        );
        Some(token)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = name;
        None
    }
}

fn save_payment_recovery(recovery: &PaymentRecovery) -> bool {
    let Ok(value) = serde_json::to_string(recovery) else {
        return false;
    };
    let key = format!("brunch-payment-recovery-{}", recovery.order.id);
    storage_set(&key, Some(&value), true);
    storage_set("brunch-payment-recovery", Some(&value), true);
    storage_get(&key, true).as_deref() == Some(value.as_str())
        && storage_get("brunch-payment-recovery", true).as_deref() == Some(value.as_str())
}

fn redirect_to_payment(url: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let Ok(parsed) = web_sys::Url::new(url) else {
            return false;
        };
        // Repeat the server's allowlist before navigating away from the shop.
        if parsed.protocol() != "https:"
            || !matches!(
                parsed.hostname().as_str(),
                "checkout.teya.com" | "checkout.teya.xyz"
            )
            || !parsed.port().is_empty()
            || !parsed.username().is_empty()
            || !parsed.password().is_empty()
        {
            return false;
        }
        web_sys::window().is_some_and(|w| w.location().set_href(url).is_ok())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = url;
        false
    }
}

impl Shop {
    fn pending_attempt(self) -> bool {
        self.attempt.get_untracked().is_some()
            || storage_get("brunch-order-attempt", true).is_some()
    }
    fn choose_fulfillment(self, pickup: bool) {
        if self.submitting.get_untracked() {
            return;
        }
        self.pickup.set(pickup);
        if pickup {
            self.pickup_selection
                .set(Some(self.settings.get_untracked().pickup));
        }
        self.order_error.set(String::new());
        self.accepted_terms.set(None);
    }
    fn can_fulfill(self) -> bool {
        if self.pickup.get() {
            self.ready.get()
                && (self.settings.get().pickup.enabled || self.pending_attempt())
                && self.pickup_selection.get().is_some()
        } else {
            self.delivers_to_postal()
        }
    }

    fn terms_accepted(self) -> bool {
        !self.terms_loading.get()
            && self.terms_error.get().is_empty()
            && self.terms.with(|terms| {
                terms
                    .as_ref()
                    .is_some_and(|terms| Some(terms.version) == self.accepted_terms.get())
            })
    }
    fn load_terms(self) {
        if self.terms_loading.get_untracked() {
            return;
        }
        self.terms_loading.set(true);
        self.terms_error.set(String::new());
        spawn_local(async move {
            match api::get::<Terms>("/api/terms").await {
                Ok(terms)
                    if terms.version > 0
                        && !terms.title.trim().is_empty()
                        && !terms.content.trim().is_empty() =>
                {
                    if self
                        .terms
                        .with_untracked(|previous| previous.as_ref().map(|value| value.version))
                        != Some(terms.version)
                    {
                        self.accepted_terms.set(None);
                    }
                    self.terms.set(Some(terms));
                }
                _ => {
                    self.terms.set(None);
                    self.accepted_terms.set(None);
                    self.terms_error.set("Obchodní podmínky se nepodařilo načíst. Před odesláním objednávky zkuste načtení znovu. Košík zůstává uložený.".into());
                }
            }
            self.terms_loading.set(false);
        });
    }
    fn subtotal(self) -> i64 {
        catalog::subtotal(&self.cart.get(), &self.products.get())
    }
    fn banquet_quantity(self) -> u32 {
        catalog::banquet_quantity(&self.cart.get(), &self.products.get())
    }
    fn banquet_missing(self) -> u32 {
        catalog::banquet_missing(
            &self.cart.get(),
            &self.products.get(),
            self.settings.get().banquet_minimum_quantity,
        )
    }
    fn banquet_can_continue(self) -> bool {
        self.banquet_missing() == 0 || self.attempt.get().is_some()
    }
    fn fee(self) -> i64 {
        if self.pickup.get() {
            0
        } else {
            catalog::delivery_fee(self.subtotal(), &self.settings.get())
        }
    }
    fn total(self) -> i64 {
        self.subtotal() + self.fee()
    }
    fn count(self, id: i64) -> u16 {
        self.cart.with(|c| c.get(&id).copied().unwrap_or(0))
    }
    fn open_product(self, id: i64) {
        self.selected_product.set(id);
        set_product_url(Some(id), true);
        open_dialog(self.product_dialog);
    }
    fn set_quantity(self, id: i64, value: u16) {
        if self.submitting.get_untracked() {
            return;
        }
        if let Some(p) = self.products.get_untracked().iter().find(|p| p.id == id) {
            if value == 0 {
                self.cart.update(|c| {
                    c.remove(&id);
                });
                self.variants.update(|v| {
                    v.remove(&id);
                });
            } else if let Some(q) = p.orderable_quantity(value) {
                self.cart.update(|c| {
                    c.insert(id, q);
                });
                if q != value {
                    self.notice.set(format!(
                        "U položky {} lze objednat {} až {} ks. Množství jsme upravili na {q} ks.",
                        p.name,
                        p.minimum(),
                        p.maximum()
                    ));
                }
            }
        }
    }
    fn add(self, id: i64) {
        if self
            .products
            .get_untracked()
            .iter()
            .find(|p| p.id == id)
            .is_some_and(|p| {
                !p.variants.is_empty()
                    && !self
                        .variants
                        .get_untracked()
                        .get(&id)
                        .is_some_and(|v| p.variants.contains(v))
            })
        {
            self.open_product(id);
            self.notice.set("Nejprve vyberte variantu svačiny.".into());
            return;
        }
        if self.submitting.get_untracked() {
            return;
        }
        if let Some(p) = self.products.get_untracked().iter().find(|p| p.id == id) {
            let current = self
                .cart
                .with_untracked(|cart| cart.get(&id).copied().unwrap_or(0));
            if let Some(count) = p.orderable_quantity(current.saturating_add(1)) {
                self.cart.update(|cart| {
                    cart.insert(id, count);
                });
                self.announcement
                    .set(format!("{} v košíku: {} ks.", p.name, count));
                self.confirmation.set(None);
                self.step.set(0);
            }
        }
    }
    fn remove(self, id: i64) {
        if self.submitting.get_untracked() {
            return;
        }
        let minimum = self.products.with_untracked(|products| {
            products
                .iter()
                .find(|p| p.id == id)
                .map_or(1, Product::minimum)
        });
        self.cart.update(|cart| {
            if let Some(n) = cart.get_mut(&id) {
                if *n <= minimum {
                    cart.remove(&id);
                } else {
                    *n -= 1;
                }
            }
        });
    }
    fn go(self, step: u8) {
        self.step.set(step);
        if let Some(title) = self.cart_title.get() {
            let _ = title.focus();
        }
        if let Some(content) = self.cart_content.get() {
            content.set_scroll_top(0);
        }
    }
    fn open_cart(self) {
        self.check_delivery_date();
        self.step
            .set(if self.confirmation.get_untracked().is_some() {
                3
            } else {
                self.step.get_untracked().min(2)
            });
        open_dialog(self.cart_dialog);
    }
    fn delivery_label(self) -> String {
        self.day
            .get()
            .parse()
            .ok()
            .map(|date| format!("{}, {}", date_label(date), self.slot.get()))
            .unwrap_or_else(|| "Vyberte požadovaný termín".into())
    }
    fn earliest_delivery_date(self) -> Option<chrono::NaiveDate> {
        earliest_date(
            self.options
                .with(|opts| opts.dates.first().and_then(|d| d.value.parse().ok())),
            self.today.get(),
        )
    }
    fn delivery_date_valid(self) -> bool {
        self.day
            .get()
            .parse()
            .ok()
            .is_some_and(|date| selectable_date(date, self.earliest_delivery_date()))
    }
    fn check_delivery_date(self) -> bool {
        let today = prague_today();
        if self.today.get_untracked() != today {
            self.today.set(today);
        }
        if !self.ready.get_untracked() {
            return false;
        }
        if self.delivery_date_valid() {
            return true;
        }
        // Keep the original payload available for an exact retry after an uncertain
        // response. The server can recover a saved order without creating a new one.
        if self.submitting.get_untracked() || self.attempt.get_untracked().is_some() {
            return false;
        }
        let previous = self.day.get_untracked();
        self.day.set(
            self.earliest_delivery_date()
                .map(|d| d.to_string())
                .unwrap_or_default(),
        );
        if !previous.is_empty() {
            self.notice.set("Původně zvolený den už není dostupný. Vybrali jsme nejbližší pracovní den. Zkontrolujte prosím termín převzetí.".into());
            if self.step.get_untracked() == 2 {
                self.go(1);
            }
        }
        false
    }
    fn available_slots(self) -> Vec<String> {
        if self.pickup.get() {
            return self
                .pickup_selection
                .get()
                .map(|p| p.slots)
                .unwrap_or_default();
        }
        let postal = self.details.postal.get();
        self.options.with(|options| {
            options
                .postal_slots
                .get(&postal)
                .unwrap_or(&options.slots)
                .clone()
        })
    }
    fn delivery_info(self) -> String {
        if self.pickup.get() {
            return "Osobní odběr zdarma.".into();
        }
        let s = self.settings.get();
        if !self.ready.get() {
            return "Načítáme cenu dopravy…".into();
        }
        match s.free_delivery_from_cents {
            Some(limit) => format!(
                "Doručení {}. Od {} zdarma.",
                money(s.delivery_fee_cents),
                money(limit)
            ),
            None => format!("Doručení {}.", money(s.delivery_fee_cents)),
        }
    }
    fn delivers_to_postal(self) -> bool {
        self.ready.get()
            && catalog::delivery_available(&self.details.postal.get(), &self.settings.get())
    }
    fn refresh(self) {
        self.loading.set(true);
        self.error.set(String::new());
        self.ready.set(false);
        spawn_local(async move {
            let result = async {
                let p = api::get::<Vec<Product>>("/api/products").await?;
                let s = api::get::<Settings>("/api/settings").await?;
                let d = api::get::<DeliveryOptions>("/api/delivery-options").await?;
                Ok::<_, api::ApiError>((p, s, d))
            }
            .await;
            match result {
                Ok((products, settings, options)) => {
                    self.cart.update(|cart| {if catalog::reconcile(cart,&products) {self.notice.set("Košík jsme upravili podle aktuální dostupnosti. Zkontrolujte prosím počty.".into());}});
                    self.variants.update(|selected| {
                        selected.retain(|id, value| {
                            products
                                .iter()
                                .find(|p| p.id == *id)
                                .is_some_and(|p| p.variants.contains(value))
                        });
                    });
                    self.products.set(products);
                    if !settings.online_payment_enabled && self.payment.get_untracked() == "card" {
                        self.payment.set("card_on_delivery".into());
                    }
                    if self.pickup.get_untracked() && !self.pending_attempt() {
                        let previous = self.pickup_selection.get_untracked().map(|p| p.version);
                        if previous.is_some() && previous != Some(settings.pickup.version) {
                            self.notice.set("Změnily se údaje osobního odběru. Zkontrolujte prosím aktuální adresu a čas.".into());
                            self.accepted_terms.set(None);
                            if self.step.get_untracked() == 2 {
                                self.go(1);
                            }
                        }
                        self.pickup_selection.set(Some(settings.pickup.clone()));
                    }
                    self.settings.set(settings);
                    self.options.set(options);
                    self.ready.set(true);
                    self.check_delivery_date();
                }
                Err(err) => self.error.set(err.message),
            }
            self.loading.set(false);
        });
    }
    fn load_orders(self) {
        self.account_loading.set(true);
        self.account_error.set(String::new());
        let identity = self.user.get_untracked().map(|u| u.id);
        spawn_local(async move {
            let result = api::get::<Vec<Order>>("/api/orders/mine").await;
            if self.user.get_untracked().map(|u| u.id) != identity {
                return;
            }
            match result {
                Ok(orders) => self.orders.set(orders),
                Err(e) => {
                    if e.status == 401 {
                        self.user.set(None);
                    }
                    self.account_error.set(e.message);
                }
            }
            self.account_loading.set(false);
        });
    }
    fn open_account(self) {
        close_dialog(self.cart_dialog);
        open_dialog(self.account_dialog);
        if self.user.get_untracked().is_some() {
            self.load_orders();
        }
    }
    fn resume_payment(self) {
        if let Some(recovery) = self.payment_recovery.get_untracked() {
            self.confirmation.set(Some(recovery.order));
            self.go(3);
            open_dialog(self.cart_dialog);
            self.check_payment(false);
        }
    }
    fn check_payment(self, continue_to_gateway: bool) {
        if self.payment_busy.get_untracked() || self.payment_recovery.get_untracked().is_none() {
            return;
        }
        self.payment_busy.set(true);
        self.payment_error.set(String::new());
        self.poll_payment(continue_to_gateway, 0);
    }
    fn poll_payment(self, continue_to_gateway: bool, attempt: u8) {
        let Some(recovery) = self.payment_recovery.get_untracked() else {
            self.payment_busy.set(false);
            return;
        };
        spawn_local(async move {
            let result = api::post::<PaymentResponse>(
                &format!("/api/orders/{}/payment", recovery.order.id),
                &serde_json::json!({"idempotency_key": recovery.idempotency_key}),
            )
            .await;
            if self
                .payment_recovery
                .get_untracked()
                .as_ref()
                .map(|r| &r.order.id)
                != Some(&recovery.order.id)
            {
                self.payment_busy.set(false);
                return;
            }
            match result {
                Ok(payment) => {
                    let mut updated = recovery;
                    updated.order.status = payment.status;
                    updated.order.payment_status = payment.payment_status;
                    updated.order.payment_expires_at = payment.expires_at;
                    self.confirmation.update(|order| {
                        if order.as_ref().is_some_and(|o| o.id == updated.order.id) {
                            *order = Some(updated.order.clone());
                        }
                    });
                    self.payment_state.set(payment.payment_state.clone());
                    if updated.order.payment_status == "paid" {
                        self.payment_recovery.set(None);
                        self.payment_busy.set(false);
                        storage_set(
                            &format!("brunch-payment-recovery-{}", updated.order.id),
                            None,
                            true,
                        );
                        storage_set("brunch-payment-recovery", None, true);
                        storage_set("brunch-order-attempt", None, true);
                        self.attempt.set(None);
                        if self.user.get_untracked().is_some() {
                            self.load_orders();
                        }
                        return;
                    }
                    self.payment_recovery.set(Some(updated.clone()));
                    let persisted = save_payment_recovery(&updated);
                    if updated.order.status == "expired" {
                        // A fresh checkout must get a new idempotency key, even for identical items.
                        storage_set("brunch-order-attempt", None, true);
                        self.attempt.set(None);
                        self.payment_busy.set(false);
                        return;
                    }
                    if payment.payment_state == "ready" && continue_to_gateway {
                        self.payment_busy.set(false);
                        if !persisted {
                            self.payment_error.set("Před otevřením platební brány povolte ukládání dat pro tento web. Objednávka už existuje. Potom použijte Pokračovat k platbě.".into());
                        } else if !payment
                            .checkout_url
                            .as_deref()
                            .is_some_and(redirect_to_payment)
                        {
                            self.payment_error.set("Platební bránu se nepodařilo otevřít. Objednávka je uložená, zkuste znovu pokračovat k platbě.".into());
                        }
                        return;
                    }
                    if payment.payment_state == "pending"
                        || (!continue_to_gateway && payment.payment_state == "ready")
                    {
                        if attempt < 59 {
                            set_timeout(
                                move || self.poll_payment(continue_to_gateway, attempt + 1),
                                std::time::Duration::from_secs(2),
                            );
                            return;
                        }
                        self.payment_error.set("Potvrzení platby zatím nedorazilo. Stav můžete znovu ověřit. Stejnou objednávku znovu nevytvářejte.".into());
                    }
                }
                Err(error) => {
                    self.payment_error.set(format!("{} Objednávka je uložená. Ověřte znovu stav platby, nevytvářejte stejnou objednávku znovu.", error.message));
                }
            }
            self.payment_busy.set(false);
        });
    }
    fn submit(self) {
        if self.submitting.get_untracked() || !self.ready.get_untracked() {
            return;
        }
        if !self.terms_accepted() {
            self.order_error.set(
                "Před odesláním objednávky si přečtěte obchodní podmínky a potvrďte souhlas."
                    .into(),
            );
            return;
        }
        if !self.check_delivery_date() && self.attempt.get_untracked().is_none() {
            self.go(1);
            return;
        }
        if self.payment.get_untracked() == "card"
            && self.payment_recovery.get_untracked().is_some()
            && !self
                .payment_recovery
                .get_untracked()
                .is_some_and(|recovery| recovery.order.status == "expired")
            && !matches!(
                self.payment_state.get_untracked().as_str(),
                "failed" | "expired"
            )
        {
            self.resume_payment();
            self.payment_error.set("Nejdříve ověřte nebo dokončete platbu uložené objednávky. Tím předejdete nechtěnému vytvoření druhé objednávky.".into());
            return;
        }
        if !self.can_fulfill() {
            self.go(1);
            return;
        }
        let details = self.details;
        let pickup = self.pickup.get_untracked();
        let mut body = OrderRequest {
            fulfillment_method: pickup.then(|| "pickup".into()),
            pickup_version: pickup
                .then(|| self.pickup_selection.get_untracked().map(|p| p.version))
                .flatten(),
            team_order_id: self.team_order_id.get_untracked(),
            team_revision: self.team_revision.get_untracked(),
            billing: details
                .billing_enabled
                .get_untracked()
                .then(|| catalog::BillingInput {
                    name: details.billing_name.get_untracked().trim().into(),
                    address: details.billing_address.get_untracked().trim().into(),
                    company_id: details.billing_company_id.get_untracked().trim().into(),
                    tax_id: details.billing_tax_id.get_untracked().trim().into(),
                }),
            idempotency_key: String::new(),
            email: details.email.get_untracked().trim().into(),
            name: details.name.get_untracked().trim().into(),
            phone: details.phone.get_untracked().trim().into(),
            organization: details.organization.get_untracked().trim().into(),
            street: if pickup {
                String::new()
            } else {
                details.street.get_untracked().trim().into()
            },
            city: if pickup {
                String::new()
            } else {
                details.city.get_untracked().trim().into()
            },
            postal_code: if pickup {
                String::new()
            } else {
                catalog::normalize_postal_code(&details.postal.get_untracked())
            },
            notes: details.notes.get_untracked().trim().into(),
            delivery_date: self.day.get_untracked(),
            delivery_slot: self.slot.get_untracked(),
            payment_method: self.payment.get_untracked(),
            items: self
                .cart
                .get_untracked()
                .into_iter()
                .map(|(product_id, quantity)| RequestedItem {
                    variant: self.variants.get_untracked().get(&product_id).cloned(),
                    product_id,
                    quantity,
                })
                .collect(),
            expected_total_cents: self.total(),
            terms_version: self.accepted_terms.get_untracked(),
        };
        let saved = self.attempt.get_untracked().or_else(|| {
            storage_get("brunch-order-attempt", true)
                .and_then(|v| serde_json::from_str::<(String, String)>(&v).ok())
        });
        let retry_key = saved.as_ref().and_then(|(previous, key)| {
            body.recover_terms_for_retry(previous).then(|| key.clone())
        });
        let signature = serde_json::to_string(&body).unwrap_or_default();
        if self.banquet_missing() > 0 && retry_key.is_none() {
            self.order_error.set(format!(
                "Přidejte ještě {} ks banketního pečiva. Společné minimum je {} ks a druhy můžete kombinovat.",
                self.banquet_missing(),
                self.settings.get_untracked().banquet_minimum_quantity
            ));
            self.go(0);
            return;
        }
        if !self.delivery_date_valid() && retry_key.is_none() {
            self.order_error
                .set("Vyberte nový termín převzetí, nejdříve následující pracovní den.".into());
            self.go(1);
            return;
        }
        body.idempotency_key = match retry_key {
            Some(key) => key,
            _ => match new_key() {
                Some(key) => key,
                None => {
                    self.order_error.set(
                        "Pro dokončení objednávky použijte zabezpečené připojení HTTPS.".into(),
                    );
                    return;
                }
            },
        };
        self.attempt
            .set(Some((signature.clone(), body.idempotency_key.clone())));
        storage_set(
            "brunch-order-attempt",
            Some(&serde_json::to_string(&(signature, &body.idempotency_key)).unwrap_or_default()),
            true,
        );
        self.submitting.set(true);
        self.order_error.set(String::new());
        spawn_local(async move {
            match api::post::<Order>("/api/orders", &body).await {
                Ok(order) => {
                    let online = order.payment_method == "card" && order.payment_status != "paid";
                    if online {
                        let recovery = PaymentRecovery {
                            order: order.clone(),
                            idempotency_key: body.idempotency_key.clone(),
                        };
                        save_payment_recovery(&recovery);
                        self.payment_recovery.set(Some(recovery));
                        self.payment_state.set("pending".into());
                    } else {
                        storage_set("brunch-order-attempt", None, true);
                        self.attempt.set(None);
                    }
                    if let Some(team) = self.team_order_id.get_untracked() {
                        storage_set(
                            &format!("brunch-team-imported-{team}"),
                            Some("ordered"),
                            true,
                        );
                    }
                    self.team_order_id.set(None);
                    self.team_revision.set(None);
                    self.confirmation.set(Some(order));
                    self.accepted_terms.set(None);
                    self.cart.set(Cart::new());
                    storage_set("brunch-checkout-draft", None, true);
                    self.notice.set(String::new());
                    self.go(3);
                    self.refresh();
                    if online {
                        self.check_payment(true);
                    }
                }
                Err(err) => {
                    if err.status == 409 {
                        self.attempt.set(None);
                        storage_set("brunch-order-attempt", None, true);
                        if body.terms_version != self.accepted_terms.get_untracked() {
                            self.accepted_terms.set(None);
                        }
                        self.order_error.set(format!("{} Zkontrolujte prosím aktualizovanou nabídku, cenu i termín a poté objednávku znovu potvrďte.",err.message));
                        self.refresh();
                        self.load_terms();
                    } else if err.status == 400 {
                        self.attempt.set(None);
                        storage_set("brunch-order-attempt", None, true);
                        self.order_error.set(err.message);
                        self.refresh();
                        self.load_terms();
                    } else {
                        self.order_error.set(format!("{} Pokud se spojení přerušilo, opakujte stejnou objednávku tímto tlačítkem. Opakované odeslání stejné objednávky ji nevytvoří podruhé.",err.message));
                    }
                }
            }
            self.submitting.set(false);
        });
    }
}

#[component]
fn Home() -> impl IntoView {
    let shop = Shop {
        pickup: RwSignal::new(false),
        pickup_selection: RwSignal::new(None),
        cart: RwSignal::new(Cart::new()),
        products: RwSignal::new(Vec::new()),
        settings: RwSignal::new(Settings::default()),
        options: RwSignal::new(DeliveryOptions::default()),
        terms: RwSignal::new(None),
        accepted_terms: RwSignal::new(None),
        terms_loading: RwSignal::new(false),
        terms_error: RwSignal::new(String::new()),
        loading: RwSignal::new(true),
        ready: RwSignal::new(false),
        error: RwSignal::new(String::new()),
        notice: RwSignal::new(String::new()),
        announcement: RwSignal::new(String::new()),
        cart_dialog: NodeRef::new(),
        delivery_dialog: NodeRef::new(),
        product_dialog: NodeRef::new(),
        account_dialog: NodeRef::new(),
        selected_product: RwSignal::new(0),
        favorites: RwSignal::new(BTreeSet::new()),
        variants: RwSignal::new(BTreeMap::new()),
        team_order_id: RwSignal::new(None),
        team_revision: RwSignal::new(None),
        auth_sent: RwSignal::new(false),
        auth_countdown: RwSignal::new(0),
        day: RwSignal::new(String::new()),
        today: RwSignal::new(prague_today()),
        slot: RwSignal::new(String::new()),
        step: RwSignal::new(0),
        cart_title: NodeRef::new(),
        cart_content: NodeRef::new(),
        details: Details::new(),
        payment: RwSignal::new("card_on_delivery".into()),
        submitting: RwSignal::new(false),
        order_error: RwSignal::new(String::new()),
        confirmation: RwSignal::new(None),
        user: RwSignal::new(None),
        orders: RwSignal::new(Vec::new()),
        account_loading: RwSignal::new(false),
        account_error: RwSignal::new(String::new()),
        auth_message: RwSignal::new(String::new()),
        auth_busy: RwSignal::new(false),
        login_token: RwSignal::new(None),
        attempt: RwSignal::new(None),
        payment_recovery: RwSignal::new(None),
        payment_busy: RwSignal::new(false),
        payment_state: RwSignal::new("pending".into()),
        payment_error: RwSignal::new(String::new()),
    };
    #[cfg(target_arch = "wasm32")]
    {
        use leptos::leptos_dom::helpers::{
            set_interval_with_handle, window_event_listener_untyped,
        };
        if let Ok(timer) = set_interval_with_handle(
            move || {
                shop.check_delivery_date();
                if shop.user.get_untracked().is_some()
                    && shop.account_dialog.get().is_some_and(|d| d.open())
                {
                    shop.load_orders();
                }
            },
            std::time::Duration::from_secs(30),
        ) {
            on_cleanup(move || timer.clear());
        }
        for event in ["focus", "pageshow", "visibilitychange"] {
            let listener = window_event_listener_untyped(event, move |_| {
                shop.check_delivery_date();
            });
            on_cleanup(move || listener.remove());
        }
    }
    Effect::new(move |_| {
        if !shop.ready.get() {
            return;
        }
        if shop.pickup.get() && shop.pending_attempt() {
            return;
        }
        let slots = shop.available_slots();
        if !slots.contains(&shop.slot.get_untracked()) {
            let old = shop.slot.get_untracked();
            shop.slot.set(slots.first().cloned().unwrap_or_default());
            if !old.is_empty() && !shop.details.postal.get_untracked().is_empty() {
                shop.notice.set("Pro tuto adresu se změnil nabízený čas rozvozu. Zkontrolujte nový termín před odesláním.".into());
            }
        }
    });
    let only_favorites = RwSignal::new(false);
    let category = RwSignal::new(String::new());
    let vegan = RwSignal::new(false);
    let search = RwSignal::new(String::new());
    let restored = RwSignal::new(false);
    Effect::new(move |_| {
        if restored.get_untracked() {
            return;
        }
        if let Some(value) = storage_get("brunch-cart-v2", false) {
            shop.cart.set(catalog::parse_cart(&value));
        }
        if let Some(v) = storage_get("brunch-favorites", false)
            .and_then(|v| serde_json::from_str::<BTreeSet<i64>>(&v).ok())
        {
            shop.favorites.set(v);
        }
        if let Some(v) = storage_get("brunch-variants", false)
            .and_then(|v| serde_json::from_str::<BTreeMap<i64, String>>(&v).ok())
        {
            shop.variants.set(v);
        }
        shop.attempt.set(stored_order_attempt());
        if let Some(draft) = storage_get("brunch-checkout-draft", true)
            .and_then(|v| serde_json::from_str::<CheckoutDraft>(&v).ok())
        {
            if now_millis() - draft.saved_at < 24.0 * 60.0 * 60.0 * 1000.0 || shop.pending_attempt()
            {
                draft.apply(shop);
            } else {
                storage_set("brunch-checkout-draft", None, true);
            }
        }
        if let Some(address) = storage_get("brunch-selected-address", true)
            .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
        {
            apply_saved_address(shop, &address);
            storage_set("brunch-selected-address", None, true);
        }
        if let Some(billing) = storage_get("brunch-selected-billing", true)
            .and_then(|v| serde_json::from_str::<catalog::BillingInput>(&v).ok())
        {
            shop.details.billing_enabled.set(true);
            shop.details.billing_name.set(billing.name);
            shop.details.billing_address.set(billing.address);
            shop.details.billing_company_id.set(billing.company_id);
            shop.details.billing_tax_id.set(billing.tax_id);
            storage_set("brunch-selected-billing", None, true);
        }
        let signin_target =
            take_query_parameter("signin").filter(|v| matches!(v.as_str(), "customer" | "team"));
        if let Some(target) = signin_target.as_ref() {
            storage_set("brunch-auth-return", Some(target), true);
            open_dialog(shop.account_dialog);
        }
        restored.set(true);
        shop.load_terms();
        shop.refresh();
        let payment_return = take_query_parameter("payment_return");
        let saved_payment = payment_return
            .as_ref()
            .and_then(|id| storage_get(&format!("brunch-payment-recovery-{id}"), true))
            .or_else(|| storage_get("brunch-payment-recovery", true));
        if let Some(recovery) =
            saved_payment.and_then(|value| serde_json::from_str::<PaymentRecovery>(&value).ok())
        {
            let returned_to_order = payment_return.as_deref() == Some(recovery.order.id.as_str());
            shop.payment_recovery.set(Some(recovery));
            if returned_to_order {
                shop.resume_payment();
            } else if payment_return.is_some() {
                shop.notice.set("Návrat z platební brány neodpovídá uložené objednávce. Stav platby ověřte v původním okně prohlížeče nebo se obraťte na obsluhu.".into());
            }
        } else if payment_return.is_some() {
            shop.notice.set("Stav této platby zde nelze ověřit. Otevřete původní okno prohlížeče, zkontrolujte potvrzovací e-mail nebo kontaktujte obsluhu. Stejnou objednávku znovu nevytvářejte.".into());
        }
        if let Some(token) = take_login_token() {
            shop.login_token.set(Some(token));
            open_dialog(shop.account_dialog);
        } else {
            spawn_local(async move {
                if let Ok(response) = api::get::<UserResponse>("/api/auth/me").await {
                    if signin_target.is_some() {
                        return_after_login(&response.user.email);
                    }
                    if shop.details.email.get_untracked().trim().is_empty() {
                        shop.details.email.set(response.user.email.clone());
                    }
                    shop.user.set(Some(response.user));
                }
            });
        }
    });
    Effect::new(move |_| {
        if restored.get() {
            if shop.cart.get().is_empty() {
                shop.team_order_id.set(None);
                shop.team_revision.set(None);
            }
            storage_set(
                "brunch-cart-v2",
                Some(&serde_json::to_string(&shop.cart.get()).unwrap_or_default()),
                false,
            );
        }
    });
    Effect::new(move |_| {
        if restored.get() {
            storage_set(
                "brunch-favorites",
                Some(&serde_json::to_string(&shop.favorites.get()).unwrap_or_default()),
                false,
            );
            storage_set(
                "brunch-variants",
                Some(&serde_json::to_string(&shop.variants.get()).unwrap_or_default()),
                false,
            );
            let draft = CheckoutDraft::read(shop);
            if shop.confirmation.get().is_none() && !shop.cart.get().is_empty() {
                storage_set(
                    "brunch-checkout-draft",
                    Some(&serde_json::to_string(&draft).unwrap_or_default()),
                    true,
                );
            } else {
                storage_set("brunch-checkout-draft", None, true);
            }
        }
    });
    let imported = RwSignal::new(false);
    Effect::new(move |_| {
        if shop.ready.get() && !imported.get_untracked() {
            imported.set(true);
            if let Some(id) = product_from_url()
                && shop.products.get_untracked().iter().any(|p| p.id == id)
            {
                shop.selected_product.set(id);
                open_dialog(shop.product_dialog);
            }
            import_team_cart(shop);
            if take_query_parameter("checkout").as_deref() == Some("1") {
                shop.open_cart();
            }
        }
    });
    #[cfg(target_arch = "wasm32")]
    {
        let listener =
            leptos::leptos_dom::helpers::window_event_listener_untyped("popstate", move |_| {
                if let Some(id) = product_from_url() {
                    shop.selected_product.set(id);
                    open_dialog(shop.product_dialog);
                } else {
                    close_dialog(shop.product_dialog);
                }
            });
        on_cleanup(move || listener.remove());
        if let Ok(timer) = leptos::leptos_dom::helpers::set_interval_with_handle(
            move || {
                shop.auth_countdown.update(|n| *n = n.saturating_sub(1));
            },
            std::time::Duration::from_secs(1),
        ) {
            on_cleanup(move || timer.clear());
        }
    }
    let filtered = move || {
        shop.products
            .get()
            .into_iter()
            .filter(|p| {
                (category.get().is_empty() || category.get() == p.category)
                    && (!vegan.get() || p.vegan)
                    && (!only_favorites.get() || shop.favorites.get().contains(&p.id))
                    && catalog::search_key(&format!(
                        "{} {} {}",
                        p.name, p.description, p.ingredients
                    ))
                    .contains(&catalog::search_key(&search.get()))
            })
            .collect::<Vec<_>>()
    };
    view! {
        <a class="skip-link" href="#nabidka">"Přejít na nabídku"</a>
        <header class="site-header"><div class="header-inner page-width">
            <Brand/>
            <nav class="desktop-nav" aria-label="Hlavní navigace"><a href="#nabidka" class="active">"Naše nabídka"</a><a href="#jak-to-funguje">"Jak to funguje"</a></nav>
            <div class="header-actions"><button class="account-button" aria-label=move || if shop.user.get().is_some() {"Můj účet"} else {"Přihlásit"} on:click=move |_| shop.open_account()><Icon name="person"/><span>{move || if shop.user.get().is_some() {"Můj účet"} else {"Přihlásit"}}</span></button><button class="header-cart" on:click=move |_| shop.open_cart() aria-label=move || format!("Otevřít košík, {} kusů",catalog::quantity(&shop.cart.get()))><Icon name="bag"/><span class="cart-label">"Košík"</span><span class="cart-count">{move ||catalog::quantity(&shop.cart.get())}</span></button></div>
        </div></header>
        <main>
            <Show when=move ||shop.payment_recovery.get().is_some()><div class="page-width api-message" role="status"><p>"Máte uloženou objednávku s online platbou. Ověřte její stav nebo pokračujte v platbě."</p><button class="button secondary" on:click=move |_|shop.resume_payment()>"Otevřít uloženou objednávku"</button></div></Show>
            <section class="hero page-width" aria-labelledby="hero-title"><div class="hero-copy"><div class="eyebrow"><span class="status-dot"></span>"Z PEKÁRNY MYSTERIUM GAUDI"</div><h1 id="hero-title">"Dobrá pauza."<br/><span>"Lepší den."</span></h1><p>"Jedinečné houstičky z pekárny Mysterium Gaudi, pečené podle originální domácí receptury. Poctivý základ našich svačin, doplněný čerstvými surovinami. Až k vám do kanceláře."</p><a class="button primary hero-cta" href="#nabidka">"Vybrat svačiny"</a></div><div class="hero-visual"><HeroPhoto shop/><div class="photo-caption"><span class="caption-icon"><Icon name="sun"/></span><div><strong>"Malá pauza. Velká radost."</strong><span>"Dobré věci si zaslouží svůj čas."</span></div></div><span class="photo-index">"01 / DOBRÉ RÁNO"</span></div></section>
            <div class="page-width"><DeliveryAreaSummary shop/><PickupTeaser shop/><section class="delivery-strip" aria-label="Nastavení převzetí"><button class="delivery-selector" on:click=move |_| {shop.check_delivery_date();open_dialog(shop.delivery_dialog);}><Icon name="pin"/><span><small>{move ||if shop.pickup.get() {"OSOBNÍ ODBĚR"} else {"VAŠE ADRESA"}}</small><strong>{move || if shop.pickup.get() {shop.pickup_selection.get().map(|p|p.name).unwrap_or_else(||"Osobní odběr".into())} else if shop.details.postal.get().is_empty() {"Kam to přivezeme?".into()} else {format!("PSČ {}",shop.details.postal.get())}}</strong></span><Icon name="chevron"/></button><button class="delivery-selector" on:click=move |_| {shop.check_delivery_date();open_dialog(shop.delivery_dialog);}><Icon name="clock"/><span><small>"POŽADOVANÝ TERMÍN"</small><strong>{move ||shop.delivery_label()}</strong></span><Icon name="chevron"/></button><div class="delivery-price"><Icon name="truck"/><span>{move ||shop.delivery_info()}</span></div></section><div class="benefits"><span><Icon name="leaf"/>"Suroviny, které znáte"</span><span><Icon name="sun"/>"Připraveno na vaše ráno"</span><span><Icon name="bag"/>"Pro vás i celý tým"</span></div></div>
            <section class="catalog page-width" id="nabidka" aria-labelledby="catalog-title"><div class="section-heading"><div><p class="eyebrow">"VAŠE DALŠÍ OBLÍBENÁ SVAČINA"</p><h2 id="catalog-title">"Na co máte chuť?"</h2></div><p class="catalog-note">"Originální houstičky. Domácí receptura. Poctivá chuť."</p></div>
                <div class="catalog-team"><span>"Objednáváte s kolegy?"</span><a href="/team/">"Společný košík"<Icon/></a></div>
                <div class="filter-bar"><div class="categories" role="group" aria-label="Kategorie svačin">{move || {let mut names = vec![String::new()]; for p in shop.products.get() {if !p.category.is_empty() && !names.contains(&p.category) {names.push(p.category);}} names.into_iter().map(move |name| {let active=name.clone(); let pressed=name.clone(); let choose=name.clone(); let label=if name.is_empty() {"Vše".to_owned()} else if name=="Vše" {"Kategorie Vše".to_owned()} else {name}; view! {<button class:active=move ||category.get()==active aria-pressed=move ||category.get()==pressed on:click=move |_|category.set(choose.clone())>{label}</button>}}).collect_view()}}</div><label class="vegan-filter"><input type="checkbox" prop:checked=move ||vegan.get() on:change=move |ev|vegan.set(event_target_checked(&ev))/><Icon name="leaf"/>"Jen rostlinné"</label></div>
                <label class="favorite-filter"><input type="checkbox" prop:checked=move ||only_favorites.get() on:change=move |ev|only_favorites.set(event_target_checked(&ev))/>{"Moje oblíbené"}</label><label class="catalog-search"><Icon name="bag"/><input type="search" aria-label="Hledat v nabídce" placeholder="Najít oblíbenou svačinu…" prop:value=move ||search.get() on:input=move |ev|search.set(event_target_value(&ev))/></label>
                <Show when=move ||shop.loading.get()><p class="shop-status" role="status">"Načítáme čerstvou nabídku…"</p></Show>
                <Show when=move ||!shop.error.get().is_empty()><div class="api-message error" role="alert"><p>{move ||shop.error.get()}</p><button class="button secondary" on:click=move |_|shop.refresh()>"Zkusit znovu"</button></div></Show>
                <Show when=move ||!shop.notice.get().is_empty()><div class="api-message" role="status">{move ||shop.notice.get()}</div></Show>
                <div class="result-meta" role="status">{move ||if shop.ready.get() {format!("{} položek v nabídce",filtered().len())} else {String::new()}}</div>
                <div class="product-grid"><For each=filtered key=|p| (p.id,p.version,p.price_cents,p.stock) children=move |product| view! {<ProductCard product shop/>}/></div>
                <Show when=move ||shop.ready.get() && filtered().is_empty()><div class="empty-results"><Icon name="leaf"/><h3>"Tady je zatím prázdno."</h3><p>"Zkuste jinou kategorii nebo hledání. Nabídku průběžně doplňujeme."</p><button class="button secondary" on:click=move |_|{category.set(String::new());vegan.set(false);only_favorites.set(false);search.set(String::new());}>"Zobrazit celou nabídku"</button></div></Show>
                <p class="catalog-disclaimer">"Konečné ceny včetně případné DPH. Fotografie jsou ilustrační."</p>
            </section>
            <section class="how-section page-width" id="jak-to-funguje" aria-labelledby="how-title"><div class="section-heading"><div><p class="eyebrow">"PAUZA BEZ STAROSTÍ"</p><h2 id="how-title">"Vy máte svůj den. My svačinu."</h2></div></div><div class="steps-grid"><article><span class="step-number">"01"</span><h3>"Vyberte si podle chuti"</h3><p>"Slané, sladké nebo něco do misky. Složení i alergeny najdete u každé svačiny."</p></article><article><span class="step-number">"02"</span><h3>"Vyberte převzetí a termín"</h3><p>{move ||if shop.settings.get().pickup.enabled {"Zvolte doručení na adresu nebo osobní odběr. V kalendáři vyberete den a čas, který vám vyhovuje."} else {"Vyplníte adresu a kontakt pro kurýra. V kalendáři vyberete den a čas, který vám vyhovuje."}}</p></article><article><span class="step-number">"03"</span><h3>{move ||if shop.settings.get().online_payment_enabled {"Zaplaťte, jak vám to vyhovuje"} else {"Zaplaťte při převzetí"}}</h3><p>{move ||if shop.settings.get().online_payment_enabled {"Online kartou přes Teya, nebo kartou či hotově při převzetí. Objednávku s online platbou přijmeme po zaplacení."} else {"Kartou nebo hotově. Přijetí objednávky vám pošleme e-mailem, termín následně potvrdíme."}}</p></article></div></section>
        </main>
        <ContactDetails shop/>
        <footer class="site-footer page-width"><Brand class="brand footer-brand"/><p>"Dobrá pauza začíná tady."</p><button class="text-button" on:click=move |_|shop.open_account()>"Můj účet a objednávky"</button><a class="terms-link" href="/obchodni-podminky/">"Obchodní podmínky"</a><a class="admin-link" href="/admin/">"Administrace"</a><span>"2026"</span></footer>
        <div class="mobile-cart" class:has-items={move ||catalog::quantity(&shop.cart.get())>0}><button class="button primary" on:click=move |_|shop.open_cart()><span><Icon name="bag"/>"Zobrazit košík"<span class="cart-count">{move ||catalog::quantity(&shop.cart.get())}</span></span><strong>{move ||if shop.ready.get() {money(shop.total())} else {"Načítáme…".into()}}<Icon/></strong></button></div>
        <div class="sr-only" aria-live="polite" aria-atomic="true">{move ||shop.announcement.get()}</div>
        <DeliveryDialog shop/><ProductDialog shop/><CartDialog shop/><AccountDialog shop/>
    }
}

fn product_image(product: &Product) -> impl IntoView + use<> {
    let path = product.image.trim();
    if !path.is_empty() && !path.starts_with("//") && !path.contains([':', '\\']) {
        return leptos::either::Either::Left(view! {
            <img class="catalog-photo" src=path.to_owned() alt="" loading="lazy" decoding="async"/>
        });
    }
    let class = match product.name.as_str() {
        "Avokádový kváskový sendvič" => "snack-image snack-0",
        "Jogurt, granola & lesní ovoce" => "snack-image snack-1",
        "Hummus bowl se zeleninou" => "snack-image snack-2",
        "Banánový chlebíček" => "snack-image snack-3",
        "Celozrnný bagel s lučinou" => "snack-image snack-4",
        "Pomeranč, mrkev & zázvor" => "snack-image snack-5",
        _ => "product-placeholder",
    };
    leptos::either::Either::Right(view! { <span class=class aria-hidden="true"></span> })
}
#[component]
fn HeroPhoto(shop: Shop) -> impl IntoView {
    let failed_source = RwSignal::new(None::<String>);
    let source = move || {
        let requested = shop.settings.with(|settings| settings.hero_image.clone());
        if requested.is_empty() || failed_source.get().as_deref() == Some(requested.as_str()) {
            catalog::DEFAULT_HERO_IMAGE.to_owned()
        } else {
            requested
        }
    };
    view! {
        <img src=source
            alt=move || if source() == catalog::DEFAULT_HERO_IMAGE {
                catalog::DEFAULT_HERO_ALT.to_owned()
            } else {
                shop.settings.with(|settings| settings.hero_image_alt.clone())
            }
            width="1536" height="1024" fetchpriority="high"
            on:error=move |event| {
                let image = event_target::<web_sys::HtmlElement>(&event);
                if let Some(failed) = image.get_attribute("src")
                    && failed != catalog::DEFAULT_HERO_IMAGE
                {
                    failed_source.set(Some(failed));
                }
            }/>
    }
}

#[component]
fn ProductCard(product: Product, shop: Shop) -> impl IntoView {
    let id = product.id;
    let image = product_image(&product);
    view! {
        <article class="product-card" class:in-cart={move ||shop.count(id)>0}>
            <button class="product-photo photo-frame" aria-label=format!("Podrobnosti: {}",product.name) on:click=move |_|{shop.open_product(id);}>
                {image}<span class="product-tag">{product.tag}</span><span class="photo-detail">"Složení a alergeny"<Icon name="plus"/></span>
            </button>
            <FavoriteButton id shop/><div class="product-info"><div class="product-meta">{(!product.category.is_empty()).then(|| view! {<span>{product.category}</span>})}<span>{product.weight}</span>{product.banquet.then(|| view! {<span>"Banketní pečivo"</span>})}{(product.minimum_quantity>1).then(|| view! {<span>{format!("Od {} ks",product.minimum_quantity)}</span>})}</div><h3><button on:click=move |_|{shop.open_product(id);}>{product.name}</button></h3><p>{product.description}</p><div class="product-bottom"><span class="product-price">{money(product.price_cents)}<small>{if product.minimum_quantity.max(1)>1 {format!("{} ks od {}",product.minimum_quantity.max(1),money(product.price_cents*i64::from(product.minimum_quantity.max(1))))}else {" / ks".into()}}</small></span><QuantityControl id shop/></div></div>
        </article>
    }
}
#[component]
fn QuantityControl(id: i64, shop: Shop) -> impl IntoView {
    let name = move || {
        shop.products.with(|p| {
            p.iter()
                .find(|p| p.id == id)
                .map(|p| p.name.clone())
                .unwrap_or_default()
        })
    };
    let bounds = move || {
        shop.products.with(|products| {
            products
                .iter()
                .find(|p| p.id == id)
                .map_or((1, 0), |p| (p.minimum(), p.maximum()))
        })
    };
    let add_label = move || match bounds() {
        (min, max) if max < min => "Vyprodáno".to_owned(),
        (min, _) if min > 1 => format!("Přidat {min} ks"),
        _ => "Přidat".to_owned(),
    };
    let add_description = move || {
        let min = bounds().0;
        if min > 1 {
            format!("Přidat {min} ks do košíku: {}", name())
        } else {
            format!("Přidat {} do košíku", name())
        }
    };
    let remove_description = move || {
        let min = bounds().0;
        if min > 1 && shop.count(id) <= min {
            format!("Odebrat z košíku: {}", name())
        } else {
            format!("Odebrat jeden kus: {}", name())
        }
    };
    view! {
        <Show when={move ||shop.count(id)>0} fallback=move ||view! {<button class="add-button" disabled=move ||{let (min,max)=bounds(); max<min || shop.submitting.get()} aria-label=add_description on:click=move |_|shop.add(id)>{add_label}<Icon name="plus"/></button>}>
            <div class="quantity-control" role="group" aria-label=move ||format!("Počet: {}",name())><button disabled=move ||shop.submitting.get() aria-label=remove_description on:click=move |_|shop.remove(id)><Icon name="minus"/></button><input type="number" inputmode="numeric" min=move ||bounds().0 max=move ||bounds().1 aria-label=move ||format!("Počet kusů: {}",name()) prop:value=move ||shop.count(id).to_string() disabled=move ||shop.submitting.get() on:change=move |ev|{if let Ok(n)=event_target_value(&ev).parse::<u16>() {shop.set_quantity(id,n);}else{event_target::<web_sys::HtmlInputElement>(&ev).set_value(&shop.count(id).to_string());shop.notice.set("Zadejte platný počet kusů.".into());}}/><button disabled={move ||shop.count(id)>=bounds().1 ||shop.submitting.get()} aria-label=move ||format!("Přidat jeden kus: {}",name()) on:click=move |_|shop.add(id)><Icon name="plus"/></button></div>
        </Show>
    }
}
#[component]
fn ProductDialog(shop: Shop) -> impl IntoView {
    view! {
        <dialog node_ref=shop.product_dialog class="product-dialog" aria-labelledby="product-title" on:close=move |_|set_product_url(None,false)><button class="icon-button floating-close" aria-label="Zavřít detail svačiny" on:click=move |_|close_dialog(shop.product_dialog)><Icon name="close"/></button>
            {move ||shop.products.get().into_iter().find(|p|p.id==shop.selected_product.get()).map(|p|view! {
                <div class="detail-photo photo-frame" role="img" aria-label=p.name.clone()>{product_image(&p)}</div><div class="detail-content"><div class="product-meta"><span>{p.tag}</span><span>{p.weight}</span>{p.banquet.then(|| view! {<span>"Banketní pečivo"</span>})}{(p.minimum_quantity>1).then(|| view! {<span>{format!("Minimální odběr {} ks",p.minimum_quantity)}</span>})}</div><h2 id="product-title">{p.name}</h2><p>{p.description}</p>{p.banquet.then(||view!{<p class="banquet-product-hint">{move ||format!("Banketní pečivo objednávejte celkem od {} ks. Do společného minima můžete kombinovat různé druhy.",shop.settings.get().banquet_minimum_quantity)}</p>})}<h3>"Co je uvnitř"</h3><p>{if p.ingredients.trim().is_empty() {"Složení zatím nebylo doplněno. Před objednáním si je prosím ověřte u obsluhy.".into()}else{p.ingredients.clone()}}</p><h3>"Alergeny"</h3><p>{catalog::allergen_label(&p.allergens)}</p><VariantSelector id=p.id shop/><a class="text-button product-permalink" href=format!("/?product={}",p.id)>"Odkaz na tuto svačinu"</a><div class="product-bottom"><strong class="product-price">{money(p.price_cents)}</strong><QuantityControl id=p.id shop/></div></div>
            })}
        </dialog>
    }
}
#[component]
fn FulfillmentChoice(shop: Shop, prefix: &'static str) -> impl IntoView {
    view! {<Show when=move ||shop.settings.get().pickup.enabled ||shop.pickup.get()><fieldset class="fulfillment-choice" disabled=move ||shop.submitting.get()><legend>"Jak si objednávku převezmete?"</legend><label class:selected=move ||!shop.pickup.get()><input type="radio" name=format!("{prefix}-fulfillment") value="delivery" prop:checked=move ||!shop.pickup.get() on:change=move |_|shop.choose_fulfillment(false)/><span><strong>"Doručení na adresu"</strong><small>{move ||money(shop.settings.get().delivery_fee_cents)}</small></span></label><label class:selected=move ||shop.pickup.get()><input type="radio" name=format!("{prefix}-fulfillment") value="pickup" prop:checked=move ||shop.pickup.get() disabled=move ||!shop.settings.get().pickup.enabled on:change=move |_|shop.choose_fulfillment(true)/><span><strong>"Osobní odběr"</strong><small>"Zdarma"{move ||if shop.settings.get().pickup.parking_free {" · Parkování zdarma"} else {""}}</small></span></label></fieldset></Show>}
}
#[component]
fn PickupAddress(location: catalog::PickupLocation) -> impl IntoView {
    view! {<div class="pickup-address"><strong>"Osobní odběr"</strong><p>{location.name}<br/>{location.street}<br/>{format!("{} {}",location.postal_code,location.city)}</p>{location.parking_free.then(||view!{<p class="pickup-parking"><Icon name="check"/>"Parkování zdarma"</p>})}{(!location.instructions.is_empty()).then(||view!{<p class="pickup-instructions">{location.instructions}</p>})}</div>}
}
#[component]
fn PickupDetails(shop: Shop) -> impl IntoView {
    view! {<Show when=move ||shop.pickup.get()>{move ||shop.pickup_selection.get().map(|p|view!{<PickupAddress location=catalog::PickupLocation{name:p.name,street:p.street,city:p.city,postal_code:p.postal_code,instructions:p.instructions,parking_free:p.parking_free}/>})}<Show when=move ||!shop.settings.get().pickup.enabled><p class="api-message error" role="alert">"Osobní odběr je nyní pozastaven. Zvolte doručení nebo se vraťte později."</p></Show></Show>}
}
#[component]
fn PickupTeaser(shop: Shop) -> impl IntoView {
    view! {<Show when=move ||shop.settings.get().pickup.enabled><section class="pickup-teaser" aria-label="Osobní odběr"><div><strong>"Vyzvedněte si objednávku osobně"</strong><p>{move ||{let p=shop.settings.get().pickup;format!("{}, {}, {} {}",p.name,p.street,p.postal_code,p.city)}}</p><Show when=move ||shop.settings.get().pickup.parking_free><span class="pickup-parking">"Parkování zdarma"</span></Show></div><button class="button secondary" on:click=move |_|{shop.choose_fulfillment(true);open_dialog(shop.delivery_dialog);}>"Zvolit osobní odběr"</button></section></Show>}
}
#[component]
fn DeliveryFields(shop: Shop, prefix: &'static str) -> impl IntoView {
    let earliest = Signal::derive(move || shop.earliest_delivery_date());
    view! {
        <DeliveryCalendar day=shop.day earliest pickup=Signal::derive(move ||shop.pickup.get())/>
        <label class="form-field checkout-time">{move ||if shop.pickup.get() {"Čas vyzvednutí"} else {"Čas doručení"}}<select required name=format!("{prefix}-time") prop:value=move ||shop.slot.get() on:change=move |ev|shop.slot.set(event_target_value(&ev))>{move ||shop.available_slots().into_iter().map(|slot|view! {<option value=slot.clone() selected=move ||shop.slot.get()==slot>{slot.clone()}</option>}).collect_view()}</select></label>
        <p class="field-hint">{move ||if shop.pickup.get() {"Osobní odběr je možný v pracovní dny od následujícího pracovního dne. Vybraný termín potvrdíme po přijetí objednávky."} else {"Rozvážíme v pracovní dny od následujícího pracovního dne. Vybraný termín potvrdíme po přijetí objednávky."}}</p>
    }
}
#[component]
fn PostalAvailability(shop: Shop, id: &'static str) -> impl IntoView {
    let message = move || {
        if !shop.ready.get() {
            return "Ověřujeme dostupnost rozvozu…";
        }
        if shop.settings.get().delivery_postal_codes.is_empty() {
            return "Rozvoz zatím není k dispozici. Zkuste to prosím později.";
        }
        let postal = shop.details.postal.get();
        if postal.is_empty() {
            return "Zadejte PSČ a ověřte si dostupnost rozvozu.";
        }
        if !catalog::valid_postal_code(&postal) {
            return "PSČ musí obsahovat pět číslic.";
        }
        if shop.delivers_to_postal() {
            "Sem rozvážíme."
        } else {
            "Na toto PSČ zatím nerozvážíme."
        }
    };
    view! {
        <p id=id class="postal-availability" role="status" class:available=move ||shop.delivers_to_postal() class:unavailable=move ||shop.ready.get() && (shop.settings.get().delivery_postal_codes.is_empty() || (catalog::valid_postal_code(&shop.details.postal.get()) && !shop.delivers_to_postal()))>{message}</p>
    }
}
#[component]
fn DeliveryDialog(shop: Shop) -> impl IntoView {
    view! {
        <dialog node_ref=shop.delivery_dialog class="delivery-dialog" aria-labelledby="delivery-title"><div class="dialog-heading"><div><p class="eyebrow">"DOBRÁ PAUZA V PRAVÝ ČAS"</p><h2 id="delivery-title">"Kam a kdy to bude?"</h2></div><button class="icon-button" aria-label="Zavřít nastavení doručení" on:click=move |_|close_dialog(shop.delivery_dialog)><Icon name="close"/></button></div><p class="dialog-intro">"Vše můžete ještě změnit v košíku."</p><form on:submit=move |ev|{ev.prevent_default();if shop.check_delivery_date() && shop.can_fulfill() {close_dialog(shop.delivery_dialog);}}><FulfillmentChoice shop prefix="destination"/><PickupDetails shop/><Show when=move ||!shop.pickup.get()><label class="form-field">"PSČ"<input name="delivery-postal" aria-describedby="delivery-postal-status" autocomplete="postal-code" inputmode="numeric" placeholder="Např. 110 00" pattern="[0-9]{5}" title="Zadejte pět číslic" required prop:value=move ||shop.details.postal.get() on:input=move |ev|update_postal_input(&ev,shop.details.postal)/></label><PostalAvailability shop id="delivery-postal-status"/></Show><DeliveryFields shop prefix="delivery"/><div class="info-note"><Icon name="truck"/><p>{move ||shop.delivery_info()}</p></div><button class="button primary full-width" type="submit" disabled=move ||!shop.can_fulfill()>"Uložit a vybírat"<Icon/></button></form></dialog>
    }
}
#[component]
fn BanquetMinimum(shop: Shop) -> impl IntoView {
    view! {
        <Show when={move ||shop.ready.get() && shop.step.get()!=3 && shop.banquet_quantity()>0}>
            <div class="banquet-minimum" class:incomplete={move ||shop.banquet_missing()>0} role="status" aria-live="polite" data-banquet-quantity=move ||shop.banquet_quantity() data-banquet-minimum=move ||shop.settings.get().banquet_minimum_quantity>
                <strong>{move ||format!("Banketní pečivo: {} z {} ks",shop.banquet_quantity(),shop.settings.get().banquet_minimum_quantity)}</strong>
                <p>{move ||if shop.banquet_missing()==0 {"Společné minimum je splněno.".into()} else if shop.attempt.get().is_some() {"Předchozí odeslání ještě nemá potvrzený výsledek. Stejnou objednávku můžete znovu ověřit. Pro novou objednávku platí aktuální minimum.".into()} else {format!("Přidejte ještě {} ks banketního pečiva. Druhy můžete libovolně kombinovat.",shop.banquet_missing())}}</p>
                <Show when={move ||shop.banquet_missing()>0 && shop.step.get()!=0 && shop.attempt.get().is_none()}>
                    <button type="button" class="text-button" on:click=move |_|shop.go(0)>"Upravit obsah košíku"</button>
                </Show>
            </div>
        </Show>
    }
}
#[component]
fn CartTotals(shop: Shop) -> impl IntoView {
    view! {<Show when=move ||shop.ready.get() fallback=move ||view! {<p class="shop-status" role="status">"Ověřujeme aktuální cenu objednávky…"</p>}><div class="cart-totals"><div><span>"Svačiny"</span><span>{move ||money(shop.subtotal())}</span></div><div><span>{move ||if shop.pickup.get() {"Osobní odběr"} else {"Doručení"}}</span><span>{move ||if shop.fee()==0 {"Zdarma".into()} else {money(shop.fee())}}</span></div><div class="grand-total"><strong>"Celkem"<small>"konečná cena"</small></strong><strong>{move ||money(shop.total())}</strong></div></div></Show>}
}
#[component]
fn CheckoutAddress(shop: Shop) -> impl IntoView {
    let d = shop.details;
    view! {<PickupDetails shop/><Show when=move ||shop.pickup.get()><p class="pickup-instructions">{move ||d.organization.get()}</p><p class="pickup-instructions">{move ||d.notes.get()}</p></Show><Show when=move ||!shop.pickup.get()><div class="checkout-address"><Icon name="pin"/><div><strong>{move ||d.name.get()}</strong><span>{move ||d.organization.get()}</span><span>{move ||d.street.get()}</span><span>{move ||format!("{} {}",d.postal.get(),d.city.get())}</span><span>{move ||d.notes.get()}</span></div></div></Show><div class="checkout-address"><Icon name="clock"/><div><strong>{move ||shop.delivery_label()}</strong><span>"Požadovaný termín, čeká na potvrzení"</span></div></div><div class="checkout-address"><Icon name="person"/><div><Show when=move ||shop.pickup.get()><strong>{move ||d.name.get()}</strong></Show><strong>{move ||d.email.get()}</strong><span>{move ||d.phone.get()}</span></div></div>}
}
#[component]
fn CheckoutItems(shop: Shop) -> impl IntoView {
    view! {<div class="checkout-review-items">{move ||shop.products.get().into_iter().filter(|p|shop.count(p.id)>0).map(|p|view! {<div><span><b>{shop.count(p.id)}"×"</b>{p.name}<small>{shop.variants.get().get(&p.id).map(|v|format!(" · {v}")).unwrap_or_default()}</small></span><strong>{money(p.price_cents*i64::from(shop.count(p.id)))}</strong></div>}).collect_view()}</div>}
}
#[component]
fn CheckoutStepper(shop: Shop) -> impl IntoView {
    view! {<ol class="checkout-stepper" aria-label="Kroky objednávky" hidden=move ||shop.step.get()==3>{[(0,"Košík"),(1,"Převzetí"),(2,"Potvrzení")].into_iter().map(move |(step,label)|view! {<li class:current=move ||shop.step.get()==step class:complete={move ||shop.step.get()>step}><button type="button" disabled={move ||step>=shop.step.get() ||shop.submitting.get()} aria-current=move ||(shop.step.get()==step).then_some("step") on:click=move |_|shop.go(step)><span class="step-dot">{step+1}</span><span>{label}</span></button></li>}).collect_view()}</ol>}
}
#[component]
fn CartDialog(shop: Shop) -> impl IntoView {
    let d = shop.details;
    view! {
        <dialog node_ref=shop.cart_dialog class="cart-dialog checkout-dialog" aria-labelledby="cart-title">
            <div class="dialog-heading"><div><p class="eyebrow">"VAŠE DOBRÁ PAUZA"</p><h2 id="cart-title" node_ref=shop.cart_title tabindex="-1">{move ||match shop.step.get() {1=>if shop.pickup.get() {"Kdy si objednávku vyzvednete?"}else{"Kam to přivezeme?"},2=>"Už jen poslední krok.",3=>match shop.confirmation.get().map(|o|o.status).as_deref() {Some("awaiting_payment")=>"Čekáme na zaplacení.",Some("cancelled")=>"Objednávka byla zrušena.",Some("expired")=>"Platnost objednávky vypršela.",_=>"Objednávku jsme přijali."},_=>"Váš košík"}}</h2></div><button class="icon-button" aria-label="Zavřít košík" on:click=move |_|close_dialog(shop.cart_dialog)><Icon name="close"/></button></div>
            <Show when={move ||shop.step.get()==3 ||catalog::quantity(&shop.cart.get())>0} fallback=move ||view! {<div class="empty-cart"><span class="empty-cart-icon"><Icon name="bag"/></span><h3>"Vaše pauza je zatím prázdná."</h3><p>"Přidejte si něco dobrého z naší nabídky."</p><Show when=move ||!shop.order_error.get().is_empty()><div class="api-message error" role="alert">{move ||shop.order_error.get()}</div></Show><button class="button primary" on:click=move |_|close_dialog(shop.cart_dialog)>"Jdu si vybrat"<Icon/></button></div>}>
                <CheckoutStepper shop/>
                <div class="checkout-body" node_ref=shop.cart_content>
                    <BanquetMinimum shop/>
                    <Show when=move ||!shop.notice.get().is_empty()><div class="api-message" role="status">{move ||shop.notice.get()}</div></Show>
                    <Show when=move ||!shop.error.get().is_empty()><div class="api-message error" role="alert"><p>{move ||shop.error.get()}</p><button class="button secondary" on:click=move |_|shop.refresh()>"Obnovit nabídku"</button></div></Show>
                    <section data-checkout-step="cart" hidden=move ||shop.step.get()!=0 aria-label="Obsah košíku">
                        <Show when=move ||!shop.pickup.get() && shop.settings.get().free_delivery_from_cents.is_some()><div class="free-delivery"><Icon name="truck"/><p>{move ||{let limit=shop.settings.get().free_delivery_from_cents.unwrap_or(0);if shop.subtotal()>=limit {"Doručení máte od nás. Dobrou chuť!".into()} else {format!("Ještě {} a doručení máte zdarma.",money(limit-shop.subtotal()))}}}</p><progress max=move ||shop.settings.get().free_delivery_from_cents.unwrap_or(1).max(1) value=move ||shop.subtotal() aria-label="Do dopravy zdarma"/></div></Show>
                        <FulfillmentChoice shop prefix="cart"/><PickupDetails shop/><div class="cart-items"><For each={move ||shop.products.get().into_iter().filter(|p|shop.count(p.id)>0).collect::<Vec<_>>()} key=|p|(p.id,p.version,p.price_cents) children=move |p|view! {<article class="cart-item"><div class="cart-item-photo photo-frame" role="img" aria-label=p.name.clone()>{product_image(&p)}</div><div class="cart-item-info"><h3>{p.name.clone()}</h3>{p.banquet.then(||view!{<small class="banquet-product-label">"Banketní pečivo"</small>})}<VariantSelector id=p.id shop/><p>{p.weight}" · "{money(p.price_cents)}" / ks"</p><div class="cart-item-actions"><QuantityControl id=p.id shop/><button class="text-button" aria-label=format!("Odebrat celou položku: {}",p.name) disabled=move ||shop.submitting.get() on:click=move |_|shop.set_quantity(p.id,0)>"Odebrat"</button><strong>{move ||money(p.price_cents*i64::from(shop.count(p.id)))}</strong></div></div></article>}/></div>
                        <CartTotals shop/><button class="button primary full-width" disabled=move ||!shop.ready.get() ||!shop.banquet_can_continue() on:click=move |_|{if shop.banquet_can_continue(){shop.go(1);}}>{move ||if shop.pickup.get() {"Pokračovat k osobnímu odběru"}else{"Pokračovat k doručení"}}<Icon/></button><p class="checkout-reassurance"><Icon name="check"/>"Bez přihlašování. Za pár chvil hotovo."</p><CartSuggestions shop/>
                    </section>
                    <section data-checkout-step="delivery" hidden=move ||shop.step.get()!=1 aria-label="Údaje pro převzetí">
                        <div class="checkout-guest"><span class="checkout-guest-icon"><Icon name="person"/></span><div><strong>{move ||if shop.user.get().is_some() {"Jste přihlášeni"} else {"Nakoupíte i bez přihlášení"}}</strong><span>"Objednávky ukládáme k zadanému e-mailu. První objednávkou vám automaticky vytvoříme účet bez hesla."</span></div></div>
                        <Show when=move ||shop.user.get().is_none()><button class="text-button optional-login" type="button" on:click=move |_|shop.open_account()>"Mám účet, přihlásit se"</button></Show>
                        <form class="checkout-form" on:submit=move |ev|{ev.prevent_default();if shop.check_delivery_date() && shop.can_fulfill() && shop.banquet_can_continue() {shop.go(2);}}>
                            <FulfillmentChoice shop prefix="checkout"/><PickupDetails shop/><Show when=move ||!shop.order_error.get().is_empty()><p class="api-message error" role="alert">{move ||shop.order_error.get()}</p></Show><Show when=move ||!shop.pickup.get()><fieldset class="checkout-fieldset"><legend>"Doručovací adresa"</legend><crate::address::StreetAddress street=d.street city=d.city postal=d.postal/><div class="form-row address-row"><label class="form-field">"Město"<input name="city" autocomplete="address-level2" pattern=r".*\S.*" required maxlength="100" prop:value=move ||d.city.get() on:input=move |ev|d.city.set(event_target_value(&ev))/></label><label class="form-field">"PSČ"<input name="postal-code" aria-describedby="checkout-postal-status" autocomplete="postal-code" inputmode="numeric" pattern="[0-9]{5}" title="Zadejte pět číslic, například 110 00" required prop:value=move ||d.postal.get() on:input=move |ev|update_postal_input(&ev,d.postal)/></label></div><PostalAvailability shop id="checkout-postal-status"/></fieldset></Show><fieldset class="checkout-fieldset"><legend>"Doplňující údaje"</legend><label class="form-field">{move ||if shop.pickup.get() {"Poznámka k odběru (nepovinné)"}else{"Pokyny pro doručení (nepovinné)"}}<textarea name="notes" rows="3" maxlength="2000" prop:value=move ||d.notes.get() on:input=move |ev|d.notes.set(event_target_value(&ev))></textarea></label><label class="form-field">"Firma (nepovinné)"<input name="organization" autocomplete="organization" maxlength="200" prop:value=move ||d.organization.get() on:input=move |ev|d.organization.set(event_target_value(&ev))/></label></fieldset><fieldset class="checkout-fieldset"><legend>"Kontaktní údaje"</legend>
                                <label class="form-field">"Jméno příjemce"<input name="name" autocomplete="name" required pattern=r".*\S.*" maxlength="200" prop:value=move ||d.name.get() on:input=move |ev|d.name.set(event_target_value(&ev))/></label>
                                <label class="form-field">{move ||if shop.pickup.get() {"Kontaktní telefon"}else{"Telefon pro kurýra"}}<input type="tel" name="tel" autocomplete="tel" required pattern=r".*[0-9].*" title="Zadejte telefonní číslo" maxlength="40" prop:value=move ||d.phone.get() on:input=move |ev|d.phone.set(event_target_value(&ev))/></label>
                                <label class="form-field">"E-mail"<input type="email" name="email" autocomplete="email" required maxlength="254" prop:value=move ||d.email.get() on:input=move |ev|d.email.set(event_target_value(&ev))/></label></fieldset>
                            <CheckoutBilling shop/>
                            <fieldset class="checkout-fieldset"><legend>"Kdy se vám to hodí?"</legend><DeliveryFields shop prefix="checkout"/></fieldset>
                            <Show when=move ||!shop.pickup.get() && !shop.can_fulfill()><p class="field-hint" role="status">"Pro pokračování ověřte PSČ v doručovací adrese."</p></Show><div class="checkout-next-total"><span>{move ||if shop.pickup.get() {"Celkem při osobním odběru"} else {"Celkem včetně doručení"}}</span><strong>{move ||money(shop.total())}</strong></div><button class="button primary full-width" type="submit" disabled=move ||!shop.can_fulfill() ||!shop.banquet_can_continue() ||shop.day.get().is_empty() ||shop.slot.get().is_empty()>"Zkontrolovat objednávku"<Icon/></button><button class="text-button checkout-back" type="button" on:click=move |_|shop.go(0)><Icon name="back"/>"Zpět do košíku"</button>
                        </form>
                    </section>
                    <section data-checkout-step="payment" hidden=move ||shop.step.get()!=2 aria-label="Platba a kontrola objednávky">
                        <div class="checkout-review"><div class="checkout-section-heading"><h3>"Vaše dobrá pauza"</h3><button class="text-button" disabled=move ||shop.submitting.get() type="button" on:click=move |_|shop.go(1)>"Upravit převzetí"</button></div><CheckoutAddress shop/><Show when=move ||!shop.pickup.get()><PostalAvailability shop id="review-postal-status"/></Show><CheckoutItems shop/><Show when=move ||d.billing_enabled.get()><p class="field-hint">{move ||format!("Fakturační údaje: {}, {}, IČO {}",d.billing_name.get(),d.billing_address.get(),d.billing_company_id.get())}</p></Show></div>
                        <fieldset class="payment-options" disabled=move ||shop.submitting.get()><legend>"Jak zaplatíte?"</legend><Show when=move ||shop.settings.get().online_payment_enabled><label class="payment-option" class:selected=move ||shop.payment.get()=="card"><input type="radio" name="payment-method" value="card" prop:checked=move ||shop.payment.get()=="card" on:change=move |_|shop.payment.set("card".into())/><Icon name="card"/><span><strong>"Online kartou"</strong><small>"Bezpečně na platební bráně Teya"</small></span></label></Show><label class="payment-option" class:selected=move ||shop.payment.get()=="card_on_delivery"><input type="radio" name="payment-method" value="card_on_delivery" prop:checked=move ||shop.payment.get()=="card_on_delivery" on:change=move |_|shop.payment.set("card_on_delivery".into())/><Icon name="card"/><span><strong>{move ||if shop.pickup.get() {"Kartou při vyzvednutí"}else{"Kartou při doručení"}}</strong><small>"Přiložíte kartu nebo telefon k terminálu"</small></span></label><label class="payment-option" class:selected=move ||shop.payment.get()=="cash_on_delivery"><input type="radio" name="payment-method" value="cash_on_delivery" prop:checked=move ||shop.payment.get()=="cash_on_delivery" on:change=move |_|shop.payment.set("cash_on_delivery".into())/><Icon name="wallet"/><span><strong>{move ||if shop.pickup.get() {"Hotově při vyzvednutí"}else{"Hotově při doručení"}}</strong><small>{move ||if shop.pickup.get() {"Zaplatíte obsluze při vyzvednutí"}else{"Zaplatíte kurýrovi při převzetí"}}</small></span></label></fieldset>
                        <CartTotals shop/>
                        <Show when=move ||!shop.order_error.get().is_empty()><div class="api-message error" role="alert">{move ||shop.order_error.get()}</div></Show>
                        <p class="field-hint checkout-order-info">{move ||if shop.payment.get()=="card" {"Odesláním vytvoříte objednávku s povinností platby a přejdete na platební bránu Teya. Objednávku přijmeme až po ověření platby a potvrdíme e-mailem. Požadovaný termín ještě potvrdí obsluha."} else if shop.pickup.get() {"Odesláním vytvoříte objednávku s povinností platby při vyzvednutí. Přijetí vám potvrdíme e-mailem. Požadovaný termín odběru ještě potvrdí obsluha."} else {"Odesláním vytvoříte objednávku s povinností platby při doručení. Přijetí vám potvrdíme e-mailem. Požadovaný termín ještě potvrdí obsluha."}}</p>
                        <CheckoutTerms shop/><button class="button primary full-width payment-submit" type="button" disabled=move ||shop.submitting.get() ||!shop.terms_accepted() ||!shop.can_fulfill() ||!shop.banquet_can_continue() ||shop.day.get().is_empty() ||shop.slot.get().is_empty() ||(shop.payment.get()=="card" && !shop.settings.get().online_payment_enabled) on:click=move |_|shop.submit()><Icon name="check"/>{move ||if shop.submitting.get() {"Odesíláme objednávku…".into()} else {format!("Objednat s povinností platby · {}",money(shop.total()))}}</button><button class="text-button checkout-back" type="button" disabled=move ||shop.submitting.get() on:click=move |_|shop.go(1)><Icon name="back"/>"Zpět k převzetí"</button>
                    </section>
                    <section data-checkout-step="confirmation" hidden=move ||shop.step.get()!=3 class="checkout-confirmation" aria-label="Potvrzení objednávky">
                        {move ||shop.confirmation.get().map(|order|view! {<div class="confirmation-intro"><span class="confirmation-mark"><Icon name="check"/></span><p class="eyebrow">"DĚKUJEME ZA OBJEDNÁVKU"</p><h3>{if order.status=="expired" {"Lhůta pro zaplacení skončila."} else {"Dobrá pauza je na dohled."}}</h3><p>"Číslo objednávky "<strong>{order.number.clone()}</strong><br/>{match order.status.as_str() {"awaiting_payment"=>"Objednávku přijmeme po ověření platby.","cancelled"=>"Objednávka je zrušena. Podrobnosti najdete u platby.","expired"=>"Objednávka nebyla včas zaplacena a je uzavřena.",_=>"Přijetí vám potvrdíme e-mailem."}}</p></div><OrderSummary order/>})}
                        <PaymentProgress shop/><p class="confirmation-note">{move ||shop.confirmation.get().map(|o|if o.status=="expired" {"Tato objednávka už nebude vyřízena. Pokud jste platbu odeslali, kontaktujte obsluhu s číslem objednávky."} else if o.status=="cancelled" {"Tato objednávka byla zrušena."} else if o.payment_method=="card" {if o.payment_status=="paid" {"Platba je ověřená. Požadovaný termín ještě potvrdíme."} else {"Objednávka je uložená a čeká na ověření platby. Nevytvářejte ji znovu."}} else if o.is_pickup() {"Platíte při vyzvednutí. Požadovaný termín odběru ještě potvrdíme."} else {"Platíte až při doručení. Požadovaný termín ještě potvrdíme."})}</p><Show when=move ||shop.user.get().is_none()><button class="button secondary full-width" on:click=move |_|shop.open_account()>"Přihlásit se a zobrazit objednávky"</button><p class="field-hint">"Objednávku už máte uloženou v účtu. Přihlaste se odkazem z e-mailu a uvidíte všechny nákupy na této adrese."</p></Show><button class="button primary full-width" on:click=move |_|{shop.confirmation.set(None);shop.go(0);close_dialog(shop.cart_dialog);}>"Zpátky na svačiny"<Icon/></button>
                    </section>
                </div>
            </Show>
        </dialog>
    }
}

#[component]
fn CheckoutTerms(shop: Shop) -> impl IntoView {
    view! {
        <div class="checkout-terms">
            <Show when=move ||shop.terms_loading.get()><p class="field-hint" role="status">"Načítáme obchodní podmínky…"</p></Show>
            <Show when=move ||!shop.terms_error.get().is_empty()>
                <div class="api-message error" role="alert"><p>{move ||shop.terms_error.get()}</p><button type="button" class="button secondary" disabled=move ||shop.terms_loading.get() on:click=move |_|shop.load_terms()>"Načíst obchodní podmínky znovu"</button></div>
            </Show>
            <Show when=move ||shop.terms.get().is_some() && !shop.terms_loading.get()>
                <label class="terms-acceptance">
                    <input type="checkbox" name="terms-accepted" required disabled=move ||shop.submitting.get()
                        prop:checked=move ||shop.terms_accepted()
                        on:change=move |ev|shop.accepted_terms.set(if event_target_checked(&ev) {shop.terms.with_untracked(|terms|terms.as_ref().map(|terms|terms.version))} else {None})/>
                    <span>"Souhlasím s "<a href=move ||shop.terms.with(|terms|terms.as_ref().map(|terms|format!("/obchodni-podminky/?version={}",terms.version)).unwrap_or_else(||"/obchodni-podminky/".into())) target="_blank" rel="noopener">"obchodními podmínkami"</a>"."</span>
                </label>
            </Show>
        </div>
    }
}

#[component]
fn PaymentProgress(shop: Shop) -> impl IntoView {
    let pending = move || {
        shop.confirmation
            .get()
            .is_some_and(|order| order.payment_method == "card" && order.payment_status != "paid")
    };
    let expired = move || {
        shop.confirmation
            .get()
            .is_some_and(|order| order.status == "expired")
    };
    view! {
        <Show when=pending>
            <div class="api-message" role="status"><p>{move ||if expired() && shop.payment_state.get() != "review" {
                "Lhůta pro zaplacení objednávky vypršela. K této objednávce už další platbu neposílejte."
            } else {match shop.payment_state.get().as_str() {
                "ready" => "Platba zatím není potvrzená. Pokud jste ji už dokončili, vyčkejte na ověření. Jinak můžete pokračovat na platební bránu.",
                "failed" => "Platba nebyla dokončena. Pro další postup kontaktujte obsluhu s číslem objednávky.",
                "expired" => "Platnost platby skončila. Pro další postup kontaktujte obsluhu s číslem objednávky.",
                "review" => "Platbu musí ověřit obsluha. Další platbu neposílejte. Uveďte číslo objednávky při kontaktování obchodu.",
                _ => "Připravujeme nebo ověřujeme platbu. Objednávka je uložená, její stav lze ověřit i po obnovení stránky.",
            }}}</p>{move ||shop.confirmation.get().filter(|order|order.status == "awaiting_payment").and_then(|order|order.payment_expires_at).map(|deadline|view! {<p class="payment-deadline">"Zaplaťte nejpozději do "<strong>{payment_deadline(&deadline)}</strong>". Potom platnost objednávky vyprší."</p>})}<Show when=move ||shop.payment_busy.get()><p>"Ověřujeme stav platby…"</p></Show></div>
            <Show when=move ||!shop.payment_error.get().is_empty()><div class="api-message error" role="alert">{move ||shop.payment_error.get()}</div></Show>
            <button class="button secondary full-width" disabled=move ||shop.payment_busy.get() on:click=move |_|shop.check_payment(false)>"Ověřit stav platby"</button>
            <Show when=move ||!expired() && shop.payment_state.get()=="ready"><button class="button primary full-width" disabled=move ||shop.payment_busy.get() on:click=move |_|shop.check_payment(true)>"Pokračovat k platbě"<Icon/></button></Show>
        </Show>
    }
}

#[component]
fn OrderSummary(order: Order) -> impl IntoView {
    let pickup = order.is_pickup();
    let status = order.status_label();
    let location = order.pickup_location.clone();
    let pickup_notes = order.notes.clone().unwrap_or_default();
    let payment = match order.payment_method.as_str() {
        "card" if order.payment_status == "paid" => "Online kartou · zaplaceno",
        "card" => "Online kartou · dosud neověřeno",
        "cash_on_delivery" if pickup => "Hotově při vyzvednutí",
        "cash_on_delivery" => "Hotově při doručení",
        _ if pickup => "Kartou při vyzvednutí",
        _ => "Kartou při doručení",
    };
    view! {
        <div class="confirmation-details"><div class="order-heading"><h4>{format!("Objednávka {}",order.number)}</h4><span class="order-status">{status}</span></div>{location.map(|location|view!{<PickupAddress location/>})}{(pickup && !pickup_notes.is_empty()).then(||view!{<p class="pickup-instructions">{pickup_notes}</p>})}<div class="checkout-address" hidden=pickup><Icon name="pin"/><div><strong>{order.street}</strong><span>{format!("{} {}",order.postal_code,order.city)}</span><span>{order.notes.unwrap_or_default()}</span></div></div><div class="checkout-address"><Icon name="clock"/><div><strong>{format!("{} · {}",chrono::NaiveDate::parse_from_str(&order.delivery_date,"%Y-%m-%d").map(date_label).unwrap_or(order.delivery_date),order.delivery_slot)}</strong><span>{if order.status=="new" {"Požadovaný termín, čeká na potvrzení"} else {"Termín objednávky"}}</span></div></div><div class="checkout-address"><Icon name="card"/><div><strong>{payment}</strong><span>{order.email}</span></div></div><div class="checkout-review-items">{order.items.into_iter().map(|item|view! {<div><span><b>{item.quantity}"×"</b>{item.name}</span><strong>{money(item.total_cents)}</strong></div>}).collect_view()}</div><div class="cart-totals"><div><span>"Svačiny"</span><span>{money(order.subtotal_cents)}</span></div><div><span>{if pickup {"Osobní odběr"}else{"Doručení"}}</span><span>{money(order.delivery_fee_cents)}</span></div><div class="grand-total"><strong>"Celkem"</strong><strong>{money(order.total_cents)}</strong></div></div></div>
    }
}
#[component]
fn AccountOrder(order: Order, shop: Shop) -> impl IntoView {
    let pickup = order.is_pickup();
    let status = order.status_label();
    let address = order.clone();
    let reorder = order.clone();
    view! {
        <article class="account-order"><details><summary><span><strong>{order.number.clone()}</strong><small>{order.delivery_date.clone()}</small></span><span class="order-status">{status}</span><strong>{money(order.total_cents)}</strong></summary><OrderSummary order=order.clone()/></details><div class="order-actions"><a class="button secondary" href=format!("/customer/#order={}",order.id)>"Doklady a změny objednávky"</a><button class="button secondary" disabled=move ||!shop.ready.get() on:click=move |_|{
            reorder_cart(shop, &reorder);
        }>"Přidat znovu do košíku"</button><button class="text-button" hidden=pickup on:click=move |_|{shop.choose_fulfillment(false);shop.details.use_address(&address);shop.auth_message.set("Doručovací adresu jsme předvyplnili pro další objednávku.".into());}>"Použít tuto adresu"</button></div></article>
    }
}
#[component]
fn AccountDialog(shop: Shop) -> impl IntoView {
    let request_link = move || {
        if shop.auth_busy.get_untracked() || shop.auth_countdown.get_untracked() > 0 {
            return;
        }
        shop.auth_busy.set(true);
        shop.account_error.set(String::new());
        shop.auth_message.set(String::new());
        let email = shop.details.email.get_untracked();
        spawn_local(async move {
            match api::post::<serde_json::Value>(
                "/api/auth/request",
                &serde_json::json!({"email":email}),
            )
            .await
            {
                Ok(_) => {
                    if let Some(target) = storage_get("brunch-auth-return", true)
                        .filter(|s| matches!(s.as_str(), "customer" | "team"))
                    {
                        let state = serde_json::json!({"target":target,"email":email.trim().to_lowercase(),"expires":now_millis()+15.0*60.0*1000.0});
                        storage_set("brunch-auth-return-shared", Some(&state.to_string()), false);
                    }
                    shop.auth_sent.set(true);
                    shop.auth_countdown.set(30);
                    shop.auth_message.set("Přihlašovací odkaz platí 15 minut. Pokud zpráva nedorazí, zkontrolujte také spam.".into());
                }
                Err(e) => shop.account_error.set(e.message),
            }
            shop.auth_busy.set(false);
        });
    };
    let verify = move || {
        if shop.auth_busy.get_untracked() {
            return;
        }
        let Some(token) = shop.login_token.get_untracked() else {
            return;
        };
        shop.auth_busy.set(true);
        shop.account_error.set(String::new());
        spawn_local(async move {
            match api::post::<UserResponse>("/api/auth/verify", &serde_json::json!({"token":token}))
                .await
            {
                Ok(response) => {
                    shop.details.email.set(response.user.email.clone());
                    shop.user.set(Some(response.user));
                    shop.login_token.set(None);
                    shop.auth_message
                        .set("Jste přihlášeni. Účet je připravený k objednávání.".into());
                    shop.load_orders();
                    return_after_login(&shop.details.email.get_untracked());
                }
                Err(e) => {
                    shop.account_error.set(e.message);
                    if e.status == 400 || e.status == 401 {
                        shop.login_token.set(None);
                    }
                }
            }
            shop.auth_busy.set(false);
        });
    };
    view! {
        <dialog node_ref=shop.account_dialog class="account-dialog" aria-labelledby="account-title"><div class="dialog-heading"><div><p class="eyebrow">"VAŠE OBLÍBENÁ PAUZA"</p><h2 id="account-title">{move ||if shop.user.get().is_some() {"Můj účet"} else {"Účet bez hesla"}}</h2></div><button class="icon-button" aria-label="Zavřít účet" on:click=move |_|close_dialog(shop.account_dialog)><Icon name="close"/></button></div>
            <Show when=move ||!shop.account_error.get().is_empty()><div class="api-message error" role="alert">{move ||shop.account_error.get()}</div></Show><Show when=move ||!shop.auth_message.get().is_empty()><div class="api-message" role="status">{move ||shop.auth_message.get()}</div></Show>
            <Show when=move ||shop.login_token.get().is_some()><div class="login-confirm"><p>"Potvrďte přihlášení jednorázovým odkazem z e-mailu."</p><button class="button primary full-width" disabled=move ||shop.auth_busy.get() on:click=move |_|verify()>{move ||if shop.auth_busy.get() {"Přihlašujeme…"} else {"Potvrdit přihlášení"}}</button></div></Show>
            <Show when=move ||shop.user.get().is_none() &&shop.login_token.get().is_none()><Show when=move ||shop.auth_sent.get()><div class="login-sent" role="status"><h3>"Zkontrolujte e-mail"</h3><p>{move ||format!("Odkaz jsme vyžádali pro {}.",shop.details.email.get())}</p><button type="button" class="text-button" on:click=move |_|{shop.auth_sent.set(false);shop.auth_message.set(String::new());}>"Opravit e-mail"</button></div></Show><div class="account-intro"><Icon name="person"/><p>"Stačí váš e-mail. Pošleme vám jednorázový odkaz pro přihlášení k objednávkám, uloženým adresám a dokladům. Žádné heslo si nemusíte pamatovat."</p></div><form on:submit=move |ev|{ev.prevent_default();request_link();}><label class="form-field">"Váš e-mail"<input type="email" autocomplete="email" name="login-email" readonly=move ||shop.auth_sent.get() required maxlength="254" prop:value=move ||shop.details.email.get() on:input=move |ev|shop.details.email.set(event_target_value(&ev))/></label><button class="button primary full-width" type="submit" disabled={move ||shop.auth_busy.get() ||shop.auth_countdown.get()>0}>{move ||if shop.auth_busy.get() {"Posíláme odkaz…".to_string()} else if shop.auth_countdown.get()>0 {format!("Poslat znovu za {} s",shop.auth_countdown.get())} else if shop.auth_sent.get() {"Poslat odkaz znovu".into()}else{"Poslat přihlašovací odkaz".into()}}</button></form><p class="field-hint">"Účet vzniká automaticky při první objednávce nebo prvním přihlášení. Uvidíte v něm i nákupy bez přihlášení na stejný e-mail, můžete je zopakovat a použít minulou adresu."</p></Show>
            <Show when=move ||shop.user.get().is_some()><div class="account-identity"><strong>{move ||shop.user.get().map(|u|u.email).unwrap_or_default()}</strong><button class="text-button" disabled=move ||shop.auth_busy.get() on:click=move |_|{
                shop.auth_busy.set(true);
                spawn_local(async move {match api::post::<serde_json::Value>("/api/auth/logout",&serde_json::json!({})).await {Ok(_) => {shop.user.set(None);shop.orders.set(Vec::new());shop.auth_message.set("Jste odhlášeni.".into());},Err(e)=>shop.account_error.set(e.message)}shop.auth_busy.set(false);});
            }>"Odhlásit"</button></div><Show when=move ||shop.user.get().is_some_and(|u|u.is_admin)><a class="button secondary full-width" href="/admin/">"Otevřít administraci"</a></Show><nav class="account-tools" aria-label="Správa účtu"><a class="button secondary" href="/customer/">"Adresy a doklady"</a><a class="button secondary" href="/team/">"Společné objednávky"</a></nav><div class="account-orders-heading"><h3>"Moje objednávky"</h3><button class="text-button" disabled=move ||shop.account_loading.get() on:click=move |_|shop.load_orders()>"Obnovit stav"</button></div><Show when=move ||shop.account_loading.get()><p class="shop-status" role="status">"Načítáme objednávky…"</p></Show><Show when=move ||!shop.account_loading.get() &&shop.orders.get().is_empty()><p class="account-empty">"Na tomto e-mailu zatím nemáte žádné objednávky."</p></Show><div class="account-orders"><For each=move ||shop.orders.get() key=|o|(o.id.clone(),o.status.clone()) children=move |order|view! {<AccountOrder order shop/>}/></div></Show>
        </dialog>
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct CheckoutDraft {
    pickup: bool,
    pickup_selection: Option<catalog::PickupSettings>,
    saved_at: f64,
    team_order_id: Option<String>,
    team_revision: Option<i64>,
    name: String,
    email: String,
    phone: String,
    organization: String,
    street: String,
    city: String,
    postal_code: String,
    notes: String,
    day: String,
    slot: String,
    step: u8,
    billing_enabled: bool,
    billing_name: String,
    billing_address: String,
    billing_company_id: String,
    billing_tax_id: String,
}
impl CheckoutDraft {
    fn read(shop: Shop) -> Self {
        let d = shop.details;
        Self {
            pickup: shop.pickup.get(),
            pickup_selection: shop.pickup_selection.get(),
            saved_at: now_millis(),
            team_order_id: shop.team_order_id.get(),
            team_revision: shop.team_revision.get(),
            name: d.name.get(),
            email: d.email.get(),
            phone: d.phone.get(),
            organization: d.organization.get(),
            street: d.street.get(),
            city: d.city.get(),
            postal_code: d.postal.get(),
            notes: d.notes.get(),
            day: shop.day.get(),
            slot: shop.slot.get(),
            step: shop.step.get().min(2),
            billing_enabled: d.billing_enabled.get(),
            billing_name: d.billing_name.get(),
            billing_address: d.billing_address.get(),
            billing_company_id: d.billing_company_id.get(),
            billing_tax_id: d.billing_tax_id.get(),
        }
    }
    fn apply(self, shop: Shop) {
        shop.pickup.set(self.pickup);
        shop.pickup_selection.set(self.pickup_selection);
        let d = shop.details;
        shop.team_order_id.set(self.team_order_id);
        shop.team_revision.set(self.team_revision);
        d.name.set(self.name);
        d.email.set(self.email);
        d.phone.set(self.phone);
        d.organization.set(self.organization);
        d.street.set(self.street);
        d.city.set(self.city);
        d.postal.set(self.postal_code);
        d.notes.set(self.notes);
        shop.day.set(self.day);
        shop.slot.set(self.slot);
        shop.step.set(self.step.min(2));
        d.billing_enabled.set(self.billing_enabled);
        d.billing_name.set(self.billing_name);
        d.billing_address.set(self.billing_address);
        d.billing_company_id.set(self.billing_company_id);
        d.billing_tax_id.set(self.billing_tax_id);
    }
}
fn now_millis() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0.0
    }
}
fn apply_saved_address(shop: Shop, value: &serde_json::Value) {
    shop.choose_fulfillment(false);
    let d = shop.details;
    for (key, signal) in [
        ("name", d.name),
        ("phone", d.phone),
        ("organization", d.organization),
        ("street", d.street),
        ("city", d.city),
        ("postal_code", d.postal),
        ("notes", d.notes),
    ] {
        if let Some(v) = value.get(key).and_then(|v| v.as_str()) {
            signal.set(v.to_owned());
        }
    }
    shop.notice
        .set("Použili jsme uloženou adresu. Zkontrolujte termín doručení.".into());
}
fn product_from_url() -> Option<i64> {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.location().href().ok())
            .and_then(|s| web_sys::Url::new(&s).ok())
            .and_then(|u| u.search_params().get("product"))
            .and_then(|v| v.parse().ok())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}
fn set_product_url(id: Option<i64>, push: bool) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(w) = web_sys::window() {
            if let Ok(url) = w
                .location()
                .href()
                .and_then(|href| web_sys::Url::new(&href))
            {
                match id {
                    Some(id) => url.search_params().set("product", &id.to_string()),
                    None => url.search_params().delete("product"),
                }
                if let Ok(h) = w.history() {
                    let _ = if push {
                        h.push_state_with_url(&wasm_null(), "", Some(&url.href()))
                    } else {
                        h.replace_state_with_url(&wasm_null(), "", Some(&url.href()))
                    };
                }
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (id, push);
    }
}
#[cfg(target_arch = "wasm32")]
fn wasm_null() -> leptos::wasm_bindgen::JsValue {
    leptos::wasm_bindgen::JsValue::NULL
}
fn return_after_login(email: &str) {
    let local = storage_get("brunch-auth-return-shared", false)
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .filter(|v| {
            v["email"]
                .as_str()
                .is_some_and(|e| e.eq_ignore_ascii_case(email.trim()))
                && v["expires"].as_f64().is_some_and(|t| t > now_millis())
        })
        .and_then(|v| v["target"].as_str().map(str::to_owned));
    if let Some(target) = storage_get("brunch-auth-return", true)
        .or(local)
        .filter(|s| matches!(s.as_str(), "customer" | "team"))
    {
        storage_set("brunch-auth-return", None, true);
        storage_set("brunch-auth-return-shared", None, false);
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(w) = web_sys::window() {
                let _ = w.location().set_href(&format!("/{target}/"));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = target;
        }
    }
}

#[derive(Deserialize)]
struct TeamImport {
    team_id: String,
    revision: i64,
    items: Vec<TeamImportItem>,
    notes: String,
}
#[derive(Deserialize)]
struct TeamImportItem {
    product_id: i64,
    quantity: u16,
    #[serde(default)]
    variant: Option<String>,
}
fn import_team_cart(shop: Shop) {
    let Some(import) = storage_get("brunch-team-import", true)
        .and_then(|s| serde_json::from_str::<TeamImport>(&s).ok())
    else {
        return;
    };
    if import.items.is_empty()
        || import.items.len() > 50
        || import.notes.chars().count() > 2000
        || import.team_id.len() > 100
    {
        shop.notice
            .set("Týmový košík není platný. Otevřete jej znovu na stránce týmu.".into());
        return;
    }
    if import.revision < 1 {
        shop.notice
            .set("Týmový výběr nemá platnou verzi. Přeneste jej znovu z týmové stránky.".into());
        return;
    }
    if !shop.cart.get_untracked().is_empty()
        && shop
            .team_order_id
            .get_untracked()
            .is_some_and(|id| id != import.team_id)
    {
        shop.notice.set("V košíku už máte jinou společnou objednávku. Nejdříve ji dokončete nebo vyprázdněte košík.".into());
        return;
    }
    let marker = format!("brunch-team-imported-{}", import.team_id);
    if storage_get(&marker, true)
        .is_some_and(|v| v == "ordered" || !shop.cart.get_untracked().is_empty())
    {
        storage_set("brunch-team-import", None, true);
        shop.notice
            .set("Tento týmový košík už byl přenesen. Zkontrolujte aktuální košík.".into());
        shop.open_cart();
        return;
    }
    let mut cart = shop.cart.get_untracked();
    let mut variants = shop.variants.get_untracked();
    let products = shop.products.get_untracked();
    for item in &import.items {
        let Some(p) = products.iter().find(|p| p.id == item.product_id) else {
            shop.notice.set(
                "Některá svačina z týmu už není dostupná. Vraťte se ke společné objednávce.".into(),
            );
            return;
        };
        let qty = cart
            .get(&p.id)
            .copied()
            .unwrap_or(0)
            .saturating_add(item.quantity);
        if p.orderable_quantity(qty) != Some(qty)
            || (!p.variants.is_empty()
                && !item
                    .variant
                    .as_ref()
                    .is_some_and(|v| p.variants.contains(v)))
            || (cart.contains_key(&p.id) && variants.get(&p.id) != item.variant.as_ref())
        {
            shop.notice.set(format!("Položku {} nelze připojit v tomto množství nebo variantě. Upravte nejprve současný košík a přenos zopakujte z týmové stránky.",p.name));
            return;
        }
        cart.insert(p.id, qty);
        if let Some(v) = &item.variant {
            variants.insert(p.id, v.clone());
        }
    }
    let existing = shop.details.notes.get_untracked();
    let notes = if existing.is_empty() {
        import.notes
    } else {
        format!("{existing}\n{}", import.notes)
    };
    if notes.chars().count() > 2000 {
        shop.notice.set("Poznámky společné objednávky jsou příliš dlouhé. Zkraťte své pokyny pro kurýra a přenos zopakujte.".into());
        return;
    }
    shop.cart.set(cart);
    shop.variants.set(variants);
    shop.details.notes.set(notes);
    shop.team_order_id.set(Some(import.team_id));
    shop.team_revision.set(Some(import.revision));
    shop.step.set(0);
    storage_set(&marker, Some("1"), true);
    storage_set("brunch-team-import", None, true);
    shop.notice.set("Týmové svačiny jsme přidali do košíku. Zkontrolujte položky, aktuální cenu a doručení. Objednávka zatím nebyla odeslána.".into());
    shop.open_cart();
}
#[component]
fn FavoriteButton(id: i64, shop: Shop) -> impl IntoView {
    view! {<button type="button" class="favorite-button" class:selected=move ||shop.favorites.get().contains(&id) aria-pressed=move ||shop.favorites.get().contains(&id).to_string() aria-label=move ||format!("{} oblíbené: {}",if shop.favorites.get().contains(&id){"Odebrat z"}else{"Přidat do"},shop.products.get().iter().find(|p|p.id==id).map(|p|p.name.clone()).unwrap_or_default()) on:click=move |_|shop.favorites.update(|f|{if !f.remove(&id){f.insert(id);}})>{move ||if shop.favorites.get().contains(&id){"♥"}else{"♡"}}</button>}
}
#[component]
fn VariantSelector(id: i64, shop: Shop) -> impl IntoView {
    let options = move || {
        shop.products
            .get()
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.variants.clone())
            .unwrap_or_default()
    };
    view! {<Show when=move ||!options().is_empty()><label class="form-field variant-selector">"Vyberte variantu"<select aria-label=move ||format!("Varianta: {}",shop.products.get().iter().find(|p|p.id==id).map(|p|p.name.clone()).unwrap_or_default()) prop:value=move ||shop.variants.get().get(&id).cloned().unwrap_or_default() on:change=move |ev|shop.variants.update(|v|{let value=event_target_value(&ev);if value.is_empty(){v.remove(&id);}else{v.insert(id,value);}})><option value="">"Zvolte náplň nebo složení"</option>{move ||options().into_iter().map(|v|view! {<option value=v.clone()>{v.clone()}</option>}).collect_view()}</select></label></Show>}
}
#[component]
fn DeliveryAreaSummary(shop: Shop) -> impl IntoView {
    view! {<div class="delivery-area-summary"><p>{move ||{let settings=shop.settings.get();if !settings.delivery_area_label.is_empty(){format!("Rozvážíme: {}",settings.delivery_area_label)}else if settings.delivery_postal_codes.is_empty(){"Dostupnost rozvozu ověřte podle PSČ.".into()}else{format!("Rozvoz na {} PSČ. Ověřte si svou adresu před výběrem.",settings.delivery_postal_codes.len())}}}</p><label class="form-field">"Ověřit rozvoz podle PSČ"<input aria-label="PSČ pro ověření rozvozu" inputmode="numeric" autocomplete="postal-code" maxlength="7" placeholder="Např. 110 00" prop:value=move ||shop.details.postal.get() on:input=move |ev|update_postal_input(&ev,shop.details.postal)/></label><PostalAvailability shop id="hero-postal-status"/></div>}
}
#[component]
fn ContactDetails(shop: Shop) -> impl IntoView {
    view! {<Show when=move ||{let s=shop.settings.get();!s.public_contact_email.is_empty()||!s.public_contact_phone.is_empty()||!s.seller_name.is_empty()}><section class="contact-section page-width" id="kontakt" aria-label="Kontakt"><h2>"Jsme tu pro vás"</h2>{move ||{let s=shop.settings.get();view!{<p>{s.seller_name}<br/>{s.seller_address}{(!s.seller_ico.is_empty()).then(||view!{<span>{format!(" · IČO {}",s.seller_ico)}</span>})}</p><div class="contact-actions">{(!s.public_contact_phone.is_empty()).then(||view!{<a href=format!("tel:{}",s.public_contact_phone.replace(' ',""))>{format!("Zavolat: {}",s.public_contact_phone)}</a>})}{(!s.public_contact_email.is_empty()).then(||view!{<a href=format!("mailto:{}",s.public_contact_email)>{format!("Napsat: {}",s.public_contact_email)}</a>})}</div>}}}</section></Show>}
}
#[component]
fn CheckoutBilling(shop: Shop) -> impl IntoView {
    let d = shop.details;
    view! {<fieldset class="checkout-fieldset"><legend>"Fakturační údaje"</legend><label class="billing-toggle"><input type="checkbox" prop:checked=move ||d.billing_enabled.get() on:change=move |ev|d.billing_enabled.set(event_target_checked(&ev))/>{"Chci uvést fakturační údaje"}</label><Show when=move ||d.billing_enabled.get()><label class="form-field">"Jméno nebo název firmy"<input name="billing-name" maxlength="200" required prop:value=move ||d.billing_name.get() on:input=move |ev|d.billing_name.set(event_target_value(&ev))/></label><label class="form-field">"Fakturační adresa"<textarea name="billing-address" maxlength="500" required prop:value=move ||d.billing_address.get() on:input=move |ev|d.billing_address.set(event_target_value(&ev))></textarea></label><div class="form-row"><label class="form-field">"IČO (nepovinné)"<input name="billing-company-id" maxlength="40" prop:value=move ||d.billing_company_id.get() on:input=move |ev|d.billing_company_id.set(event_target_value(&ev))/></label><label class="form-field">"DIČ (nepovinné)"<input name="billing-tax-id" maxlength="40" prop:value=move ||d.billing_tax_id.get() on:input=move |ev|d.billing_tax_id.set(event_target_value(&ev))/></label></div><p class="field-hint">"Vystavený doklad najdete po přihlášení u své objednávky."</p></Show></fieldset>}
}
#[component]
fn CartSuggestions(shop: Shop) -> impl IntoView {
    let suggestions = move || {
        let gap = if shop.pickup.get() {
            0
        } else {
            shop.settings.get().free_delivery_from_cents.unwrap_or(0) - shop.subtotal()
        };
        let mut products = shop
            .products
            .get()
            .into_iter()
            .filter(|p| {
                shop.count(p.id) == 0
                    && p.active
                    && p.maximum() >= p.minimum()
                    && p.variants.is_empty()
            })
            .collect::<Vec<_>>();
        products.sort_by_key(|p| {
            let price = p.price_cents * i64::from(p.minimum());
            (
                if gap > 0 {
                    (price - gap).abs()
                } else if ["napoj", "ovoce"]
                    .iter()
                    .any(|w| catalog::search_key(&p.category).contains(w))
                {
                    0
                } else {
                    1
                },
                price,
            )
        });
        products.into_iter().take(2).collect::<Vec<_>>()
    };
    view! {<Show when=move ||!suggestions().is_empty()><aside class="cart-suggestions" aria-label="Doplnění objednávky"><h3>"Ještě něco ke svačině?"</h3>{move ||suggestions().into_iter().map(|p|{let id=p.id;view!{<div><span>{p.name.clone()}<small>{format!("{} ks · {}",p.minimum(),money(p.price_cents*i64::from(p.minimum())))}</small></span><button class="button secondary" on:click=move |_|shop.add(id)>"Přidat"</button></div>}}).collect_view()}</aside></Show>}
}
fn reorder_cart(shop: Shop, order: &Order) {
    let products = shop.products.get_untracked();
    let mut cart = shop.cart.get_untracked();
    let mut variants = shop.variants.get_untracked();
    let mut omitted = false;
    for item in &order.items {
        let Some(p) = products.iter().find(|p| p.id == item.product_id) else {
            omitted = true;
            continue;
        };
        let desired = if p.variants.is_empty() {
            None
        } else {
            item.variant.clone().filter(|v| p.variants.contains(v))
        };
        if !p.variants.is_empty() && desired.is_none() {
            omitted = true;
            continue;
        }
        if cart.contains_key(&p.id) && variants.get(&p.id) != desired.as_ref() {
            shop.auth_message.set(format!(
                "V košíku už máte jinou variantu produktu {}. Nejdříve ji upravte nebo odeberte.",
                p.name
            ));
            return;
        }
        let wanted = cart
            .get(&p.id)
            .copied()
            .unwrap_or(0)
            .saturating_add(item.quantity);
        let Some(q) = p.orderable_quantity(wanted) else {
            omitted = true;
            continue;
        };
        omitted |= q != wanted;
        cart.insert(p.id, q);
        if let Some(v) = desired {
            variants.insert(p.id, v);
        } else {
            variants.remove(&p.id);
        }
    }
    shop.cart.set(cart);
    shop.variants.set(variants);
    shop.details.use_address(order);
    shop.choose_fulfillment(order.is_pickup());
    shop.confirmation.set(None);
    shop.step.set(0);
    shop.notice.set(if omitted {"Do košíku jsme přidali dostupné položky. Některé svačiny, varianty nebo počty už nejsou dostupné. Platí aktuální ceny.".into()}else{"Svačiny jsme přidali za aktuální ceny. Zkontrolujte nový termín převzetí.".into()});
    close_dialog(shop.account_dialog);
    shop.open_cart();
}
