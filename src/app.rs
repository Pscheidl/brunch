use crate::{
    api,
    catalog::{
        self, Cart, DeliveryOptions, Order, OrderRequest, PaymentRecovery, PaymentResponse,
        Product, RequestedItem, Settings, User, UserResponse, money,
    },
    icons::Icon,
};
use leptos::{html, prelude::*, task::spawn_local};

#[component]
pub fn App() -> impl IntoView {
    view! { <Home/> }
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
        }
    }
    fn use_address(self, order: &Order) {
        self.name.set(order.name.clone().unwrap_or_default());
        self.phone.set(order.phone.clone().unwrap_or_default());
        self.organization
            .set(order.organization.clone().unwrap_or_default());
        self.street.set(order.street.clone());
        self.city.set(order.city.clone());
        self.postal
            .set(catalog::normalize_postal_code(&order.postal_code));
        self.notes.set(order.notes.clone().unwrap_or_default());
    }
}
#[derive(Clone, Copy)]
struct Shop {
    cart: RwSignal<Cart>,
    products: RwSignal<Vec<Product>>,
    settings: RwSignal<Settings>,
    options: RwSignal<DeliveryOptions>,
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
    day: RwSignal<String>,
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
    fn subtotal(self) -> i64 {
        catalog::subtotal(&self.cart.get(), &self.products.get())
    }
    fn fee(self) -> i64 {
        catalog::delivery_fee(self.subtotal(), &self.settings.get())
    }
    fn total(self) -> i64 {
        self.subtotal() + self.fee()
    }
    fn count(self, id: i64) -> u16 {
        self.cart.with(|c| c.get(&id).copied().unwrap_or(0))
    }
    fn add(self, id: i64) {
        if self.submitting.get_untracked() {
            return;
        }
        if let Some(p) = self.products.get_untracked().iter().find(|p| p.id == id) {
            let max = p.stock.unwrap_or(99).clamp(0, 99) as u16;
            self.cart.update(|c| {
                let n = c.entry(id).or_default();
                *n = n.saturating_add(1).min(max);
            });
            self.announcement
                .set(format!("{} přidáno do košíku.", p.name));
            self.confirmation.set(None);
        }
    }
    fn remove(self, id: i64) {
        if self.submitting.get_untracked() {
            return;
        }
        self.cart.update(|c| {
            if let Some(n) = c.get_mut(&id) {
                *n = n.saturating_sub(1);
            }
            c.retain(|_, n| *n > 0);
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
        self.step
            .set(if self.confirmation.get_untracked().is_some() {
                3
            } else {
                0
            });
        open_dialog(self.cart_dialog);
    }
    fn delivery_label(self) -> String {
        let options = self.options.get();
        let day = self.day.get();
        options
            .dates
            .iter()
            .find(|d| d.value == day)
            .map(|d| format!("{}, {}", d.label, self.slot.get()))
            .unwrap_or_else(|| "Vyberte požadovaný termín".into())
    }
    fn delivery_info(self) -> String {
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
                    if !options
                        .dates
                        .iter()
                        .any(|d| d.value == self.day.get_untracked())
                    {
                        self.day.set(
                            options
                                .dates
                                .first()
                                .map(|d| d.value.clone())
                                .unwrap_or_default(),
                        );
                    }
                    if !options.slots.contains(&self.slot.get_untracked()) {
                        self.slot
                            .set(options.slots.first().cloned().unwrap_or_default());
                    }
                    self.products.set(products);
                    if !settings.online_payment_enabled && self.payment.get_untracked() == "card" {
                        self.payment.set("card_on_delivery".into());
                    }
                    self.settings.set(settings);
                    self.options.set(options);
                    self.ready.set(true);
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
        if !catalog::delivery_available(
            &self.details.postal.get_untracked(),
            &self.settings.get_untracked(),
        ) {
            self.go(1);
            return;
        }
        let details = self.details;
        let mut body = OrderRequest {
            idempotency_key: String::new(),
            email: details.email.get_untracked().trim().into(),
            name: details.name.get_untracked().trim().into(),
            phone: details.phone.get_untracked().trim().into(),
            organization: details.organization.get_untracked().trim().into(),
            street: details.street.get_untracked().trim().into(),
            city: details.city.get_untracked().trim().into(),
            postal_code: catalog::normalize_postal_code(&details.postal.get_untracked()),
            notes: details.notes.get_untracked().trim().into(),
            delivery_date: self.day.get_untracked(),
            delivery_slot: self.slot.get_untracked(),
            payment_method: self.payment.get_untracked(),
            items: self
                .cart
                .get_untracked()
                .into_iter()
                .map(|(product_id, quantity)| RequestedItem {
                    product_id,
                    quantity,
                })
                .collect(),
            expected_total_cents: self.total(),
        };
        let signature = serde_json::to_string(&body).unwrap_or_default();
        let saved = self.attempt.get_untracked().or_else(|| {
            storage_get("brunch-order-attempt", true)
                .and_then(|v| serde_json::from_str::<(String, String)>(&v).ok())
        });
        body.idempotency_key = match saved {
            Some((previous, key)) if previous == signature => key,
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
                    self.confirmation.set(Some(order));
                    self.cart.set(Cart::new());
                    self.notice.set(String::new());
                    self.go(3);
                    self.refresh();
                    if online {
                        self.check_payment(true);
                    }
                }
                Err(err) => {
                    if err.status == 409 {
                        self.order_error.set(format!("{} Zkontrolujte prosím aktualizovanou nabídku, cenu i termín a poté objednávku znovu potvrďte.",err.message));
                        self.refresh();
                    } else if err.status == 400 {
                        self.order_error.set(err.message);
                        self.refresh();
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
        cart: RwSignal::new(Cart::new()),
        products: RwSignal::new(Vec::new()),
        settings: RwSignal::new(Settings::default()),
        options: RwSignal::new(DeliveryOptions::default()),
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
        day: RwSignal::new(String::new()),
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
    let category = RwSignal::new("Vše".to_owned());
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
        restored.set(true);
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
                    shop.details.email.set(response.user.email.clone());
                    shop.user.set(Some(response.user));
                }
            });
        }
    });
    Effect::new(move |_| {
        if restored.get() {
            storage_set(
                "brunch-cart-v2",
                Some(&serde_json::to_string(&shop.cart.get()).unwrap_or_default()),
                false,
            );
        }
    });
    let filtered = move || {
        shop.products
            .get()
            .into_iter()
            .filter(|p| {
                (category.get() == "Vše" || category.get() == p.category)
                    && (!vegan.get() || p.vegan)
                    && format!("{} {}", p.name, p.description)
                        .to_lowercase()
                        .contains(&search.get().to_lowercase())
            })
            .collect::<Vec<_>>()
    };
    view! {
        <a class="skip-link" href="#nabidka">"Přejít na nabídku"</a>
        <div class="top-note">{move ||if shop.settings.get().online_payment_enabled {"Dobrá pauza bez registrace. Online kartou nebo při doručení."} else {"Dobrá pauza bez registrace. Kartou i hotově zaplatíte při doručení."}}</div>
        <header class="site-header"><div class="header-inner page-width">
            <a class="brand" href="./" aria-label="brunch, úvodní stránka"><span class="brand-mark"><Icon name="leaf"/></span>"brunch"<span class="brand-dot">"."</span></a>
            <nav class="desktop-nav" aria-label="Hlavní navigace"><a href="#nabidka" class="active">"Naše nabídka"</a><a href="#jak-to-funguje">"Jak to funguje"</a><a href="#do-kancelare">"Pro vaši kancelář"</a></nav>
            <div class="header-actions"><button class="account-button" on:click=move |_| shop.open_account()><Icon name="person"/><span>{move || if shop.user.get().is_some() {"Můj účet"} else {"Přihlásit"}}</span></button><button class="header-cart" on:click=move |_| shop.open_cart() aria-label=move || format!("Otevřít košík, {} kusů",catalog::quantity(&shop.cart.get()))><Icon name="bag"/><span>"Košík"</span><span class="cart-count">{move ||catalog::quantity(&shop.cart.get())}</span></button></div>
        </div></header>
        <main>
            <Show when=move ||shop.payment_recovery.get().is_some()><div class="page-width api-message" role="status"><p>"Máte uloženou objednávku s online platbou. Ověřte její stav nebo pokračujte v platbě."</p><button class="button secondary" on:click=move |_|shop.resume_payment()>"Otevřít uloženou objednávku"</button></div></Show>
            <section class="hero page-width" aria-labelledby="hero-title"><div class="hero-copy"><div class="eyebrow"><span class="status-dot"></span>"Z PEKÁRNY ROVNOU K VÁM"</div><h1 id="hero-title">"Dobrá pauza."<br/><span>"Lepší den."</span></h1><p>"Poctivé pečivo. Čerstvé suroviny. Svačiny, na které se budete těšit. Až k vám do kanceláře."</p><a class="button primary hero-cta" href="#nabidka">"Co si dnes dáte?"<Icon/></a><div class="hero-footnote"><Icon name="check"/>"Pár kliknutí. Bez registrace."</div></div><div class="hero-visual"><img src="images/brunch-spread.webp" alt="Kváskový sendvič, jogurt s granolou a hummus se zeleninou na prostřeném stole" width="1536" height="1024" fetchpriority="high"/><div class="photo-caption"><span class="caption-icon"><Icon name="sun"/></span><div><strong>"Malá pauza. Velká radost."</strong><span>"Dobré věci si zaslouží svůj čas."</span></div></div><span class="photo-index">"01 / DOBRÉ RÁNO"</span></div></section>
            <div class="page-width"><section class="delivery-strip" aria-label="Nastavení doručení"><button class="delivery-selector" on:click=move |_| open_dialog(shop.delivery_dialog)><Icon name="pin"/><span><small>"VAŠE ADRESA"</small><strong>{move || if shop.details.postal.get().is_empty() {"Kam to přivezeme?".into()} else {format!("PSČ {}",shop.details.postal.get())}}</strong></span><Icon name="chevron"/></button><button class="delivery-selector" on:click=move |_| open_dialog(shop.delivery_dialog)><Icon name="clock"/><span><small>"POŽADOVANÝ TERMÍN"</small><strong>{move ||shop.delivery_label()}</strong></span><Icon name="chevron"/></button><div class="delivery-price"><Icon name="truck"/><span>{move ||shop.delivery_info()}</span></div></section><div class="benefits"><span><Icon name="leaf"/>"Suroviny, které znáte"</span><span><Icon name="sun"/>"Připraveno na vaše ráno"</span><span><Icon name="bag"/>"Pro vás i celý tým"</span></div></div>
            <section class="catalog page-width" id="nabidka" aria-labelledby="catalog-title"><div class="section-heading"><div><p class="eyebrow">"VAŠE DALŠÍ OBLÍBENÁ SVAČINA"</p><h2 id="catalog-title">"Na co máte chuť?"</h2></div><p class="catalog-note">"Něco malého. Něco dobrého. Něco pro vás."</p></div>
                <div class="filter-bar"><div class="categories" role="group" aria-label="Kategorie svačin">{move || {let mut names = vec!["Vše".to_owned()]; for p in shop.products.get() {if !names.contains(&p.category) {names.push(p.category);}} names.into_iter().map(move |name| {let active=name.clone(); let pressed=name.clone(); let choose=name.clone(); view! {<button class:active=move ||category.get()==active aria-pressed=move ||category.get()==pressed on:click=move |_|category.set(choose.clone())>{name}</button>}}).collect_view()}}</div><label class="vegan-filter"><input type="checkbox" prop:checked=move ||vegan.get() on:change=move |ev|vegan.set(event_target_checked(&ev))/><Icon name="leaf"/>"Jen rostlinné"</label></div>
                <label class="catalog-search"><Icon name="bag"/><input type="search" aria-label="Hledat v nabídce" placeholder="Najít oblíbenou svačinu…" prop:value=move ||search.get() on:input=move |ev|search.set(event_target_value(&ev))/></label>
                <Show when=move ||shop.loading.get()><p class="shop-status" role="status">"Načítáme čerstvou nabídku…"</p></Show>
                <Show when=move ||!shop.error.get().is_empty()><div class="api-message error" role="alert"><p>{move ||shop.error.get()}</p><button class="button secondary" on:click=move |_|shop.refresh()>"Zkusit znovu"</button></div></Show>
                <Show when=move ||!shop.notice.get().is_empty()><div class="api-message" role="status">{move ||shop.notice.get()}</div></Show>
                <div class="result-meta" role="status">{move ||if shop.ready.get() {format!("{} položek v nabídce",filtered().len())} else {String::new()}}</div>
                <div class="product-grid"><For each=filtered key=|p| (p.id,p.version,p.price_cents,p.stock) children=move |product| view! {<ProductCard product shop/>}/></div>
                <Show when=move ||shop.ready.get() && filtered().is_empty()><div class="empty-results"><Icon name="leaf"/><h3>"Tady je zatím prázdno."</h3><p>"Zkuste jinou kategorii nebo hledání. Nabídku průběžně doplňujeme."</p><button class="button secondary" on:click=move |_|{category.set("Vše".into());vegan.set(false);search.set(String::new());}>"Zobrazit celou nabídku"</button></div></Show>
                <p class="catalog-disclaimer">"Konečné ceny včetně případné DPH. Fotografie jsou ilustrační."</p>
            </section>
            <section class="how-section page-width" id="jak-to-funguje" aria-labelledby="how-title"><div class="section-heading"><div><p class="eyebrow">"PAUZA BEZ STAROSTÍ"</p><h2 id="how-title">"Vy máte svůj den. My svačinu."</h2></div></div><div class="steps-grid"><article><span class="step-number">"01"</span><h3>"Vyberte si podle chuti"</h3><p>"Slané, sladké nebo něco do misky. Složení i alergeny najdete u každé svačiny."</p></article><article><span class="step-number">"02"</span><h3>"Řekněte nám kam a kdy"</h3><p>"Stačí adresa a e-mail. Vyberete požadovaný čas a doplníte třeba patro nebo zvonek."</p></article><article><span class="step-number">"03"</span><h3>{move ||if shop.settings.get().online_payment_enabled {"Zaplaťte, jak vám to vyhovuje"} else {"Zaplaťte při doručení"}}</h3><p>{move ||if shop.settings.get().online_payment_enabled {"Online kartou přes Teya, nebo kartou či hotově při doručení. Objednávku s online platbou přijmeme po zaplacení."} else {"Kartou nebo hotově. Přijetí objednávky vám pošleme e-mailem, termín následně potvrdíme."}}</p></article></div></section>
            <section class="office-section page-width" id="do-kancelare" aria-labelledby="office-title"><div class="office-banner"><div><span class="eyebrow">"DOBRÉ VĚCI SE LÉPE SDÍLÍ"</span><h2 id="office-title">"Přiberte ke svačině kolegy."</h2><p>"Každému něco podle chuti. Jedna společná objednávka, jedna cesta k vám. "{move ||shop.delivery_info()}</p></div><a class="button secondary" href="#nabidka">"Vybrat pro celý tým"<Icon/></a></div></section>
        </main>
        <footer class="site-footer page-width"><a class="brand footer-brand" href="./">"brunch."</a><p>"Dobrá pauza začíná tady."</p><button class="text-button" on:click=move |_|shop.open_account()>"Můj účet a objednávky"</button><a class="admin-link" href="/admin/">"Administrace"</a><span>"2026"</span></footer>
        <div class="mobile-cart" class:has-items={move ||catalog::quantity(&shop.cart.get())>0}><button class="button primary" on:click=move |_|shop.open_cart()><span><Icon name="bag"/>"Zobrazit košík"<span class="cart-count">{move ||catalog::quantity(&shop.cart.get())}</span></span><strong>{move ||if shop.ready.get() {money(shop.total())} else {"Načítáme…".into()}}<Icon/></strong></button></div>
        <div class="sr-only" aria-live="polite" aria-atomic="true">{move ||shop.announcement.get()}</div>
        <DeliveryDialog shop/><ProductDialog shop/><CartDialog shop/><AccountDialog shop/>
    }
}

fn photo_class(product: &Product) -> String {
    let index = match product.name.as_str() {
        "Avokádový kváskový sendvič" => Some(0),
        "Jogurt, granola & lesní ovoce" => Some(1),
        "Hummus bowl se zeleninou" => Some(2),
        "Banánový chlebíček" => Some(3),
        "Celozrnný bagel s lučinou" => Some(4),
        "Pomeranč, mrkev & zázvor" => Some(5),
        _ => None,
    };
    if !product.image.is_empty() {
        "custom-product-image".into()
    } else if let Some(index) = index {
        format!("snack-image snack-{index}")
    } else {
        "product-placeholder".into()
    }
}
fn photo_style(product: &Product) -> String {
    // Admin API accepts local asset paths only. Quote and CSS control characters are rejected here too.
    let path = &product.image;
    if path.starts_with('/')
        && !path.starts_with("//")
        && !path
            .chars()
            .any(|c| matches!(c, '\'' | '"' | '\\' | '\n' | '\r' | ')' | '('))
    {
        format!("background-image: url('{path}')")
    } else {
        String::new()
    }
}
#[component]
fn ProductCard(product: Product, shop: Shop) -> impl IntoView {
    let id = product.id;
    let class = format!("product-photo {}", photo_class(&product));
    let style = photo_style(&product);
    view! {
        <article class="product-card" class:in-cart={move ||shop.count(id)>0}>
            <button class=class style=style aria-label=format!("Podrobnosti: {}",product.name) on:click=move |_|{shop.selected_product.set(id);open_dialog(shop.product_dialog);}>
                <span class="product-tag">{product.tag}</span><span class="photo-detail">"Složení a alergeny"<Icon name="plus"/></span>
            </button>
            <div class="product-info"><div class="product-meta"><span>{product.category}</span><span>{product.weight}</span></div><h3><button on:click=move |_|{shop.selected_product.set(id);open_dialog(shop.product_dialog);}>{product.name}</button></h3><p>{product.description}</p><div class="product-bottom"><span class="product-price">{money(product.price_cents)}</span><QuantityControl id shop/></div></div>
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
    let limit = move || {
        shop.products.with(|p| {
            p.iter()
                .find(|p| p.id == id)
                .map(|p| p.stock.unwrap_or(99).clamp(0, 99) as u16)
                .unwrap_or(0)
        })
    };
    view! {
        <Show when={move ||shop.count(id)>0} fallback=move ||view! {<button class="add-button" disabled=move ||limit()==0 || shop.submitting.get() aria-label=move ||format!("Přidat {} do košíku",name()) on:click=move |_|shop.add(id)>{move ||if limit()==0 {"Vyprodáno"} else {"Přidat"}}<Icon name="plus"/></button>}>
            <div class="quantity-control" role="group" aria-label=move ||format!("Počet: {}",name())><button disabled=move ||shop.submitting.get() aria-label=move ||format!("Odebrat jeden kus: {}",name()) on:click=move |_|shop.remove(id)><Icon name="minus"/></button><span aria-live="polite">{move ||shop.count(id)}</span><button disabled={move ||shop.count(id)>=limit() ||shop.submitting.get()} aria-label=move ||format!("Přidat jeden kus: {}",name()) on:click=move |_|shop.add(id)><Icon name="plus"/></button></div>
        </Show>
    }
}
#[component]
fn ProductDialog(shop: Shop) -> impl IntoView {
    view! {
        <dialog node_ref=shop.product_dialog class="product-dialog" aria-labelledby="product-title"><button class="icon-button floating-close" aria-label="Zavřít detail svačiny" on:click=move |_|close_dialog(shop.product_dialog)><Icon name="close"/></button>
            {move ||shop.products.get().into_iter().find(|p|p.id==shop.selected_product.get()).map(|p|view! {
                <div class=format!("detail-photo {}",photo_class(&p)) style=photo_style(&p) role="img" aria-label=p.name.clone()></div><div class="detail-content"><div class="product-meta"><span>{p.tag}</span><span>{p.weight}</span></div><h2 id="product-title">{p.name}</h2><p>{p.description}</p><h3>"Co je uvnitř"</h3><p>{p.ingredients}</p><h3>"Alergeny"</h3><p>{p.allergens}</p><div class="product-bottom"><strong class="product-price">{money(p.price_cents)}</strong><QuantityControl id=p.id shop/></div></div>
            })}
        </dialog>
    }
}
#[component]
fn DeliveryFields(shop: Shop, prefix: &'static str) -> impl IntoView {
    view! {
        <div class="day-options">{move ||shop.options.get().dates.into_iter().map(|date| {
            let selected=date.value.clone();let checked=date.value.clone();let choose=date.value.clone();
            view! {<label class:selected=move ||shop.day.get()==selected><input type="radio" name=format!("{prefix}-day") value=date.value prop:checked=move ||shop.day.get()==checked on:change=move |_|shop.day.set(choose.clone())/><span>{date.label}</span></label>}
        }).collect_view()}</div>
        <label class="form-field checkout-time">"Čas doručení"<select required name=format!("{prefix}-time") prop:value=move ||shop.slot.get() on:change=move |ev|shop.slot.set(event_target_value(&ev))>{move ||shop.options.get().slots.into_iter().map(|slot|view! {<option value=slot.clone() selected=move ||shop.slot.get()==slot>{slot.clone()}</option>}).collect_view()}</select></label>
        <p class="field-hint">"Vyberte požadované časové okno. Přesný termín potvrdíme po přijetí objednávky."</p>
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
        <dialog node_ref=shop.delivery_dialog class="delivery-dialog" aria-labelledby="delivery-title"><div class="dialog-heading"><div><p class="eyebrow">"DOBRÁ PAUZA V PRAVÝ ČAS"</p><h2 id="delivery-title">"Kam a kdy to bude?"</h2></div><button class="icon-button" aria-label="Zavřít nastavení doručení" on:click=move |_|close_dialog(shop.delivery_dialog)><Icon name="close"/></button></div><p class="dialog-intro">"Vše můžete ještě změnit v košíku."</p><form on:submit=move |ev|{ev.prevent_default();if shop.delivers_to_postal() {close_dialog(shop.delivery_dialog);}}><label class="form-field">"PSČ"<input name="delivery-postal" aria-describedby="delivery-postal-status" autocomplete="postal-code" inputmode="numeric" placeholder="Např. 110 00" pattern="[0-9]{5}" title="Zadejte pět číslic" required prop:value=move ||shop.details.postal.get() on:input=move |ev|update_postal_input(&ev,shop.details.postal)/></label><PostalAvailability shop id="delivery-postal-status"/><DeliveryFields shop prefix="delivery"/><div class="info-note"><Icon name="truck"/><p>{move ||shop.delivery_info()}</p></div><button class="button primary full-width" type="submit" disabled=move ||!shop.delivers_to_postal()>"Uložit a vybírat"<Icon/></button></form></dialog>
    }
}
#[component]
fn CartTotals(shop: Shop) -> impl IntoView {
    view! {<Show when=move ||shop.ready.get() fallback=move ||view! {<p class="shop-status" role="status">"Ověřujeme aktuální cenu objednávky…"</p>}><div class="cart-totals"><div><span>"Svačiny"</span><span>{move ||money(shop.subtotal())}</span></div><div><span>"Doručení"</span><span>{move ||if shop.fee()==0 {"Zdarma".into()} else {money(shop.fee())}}</span></div><div class="grand-total"><strong>"Celkem"<small>"konečná cena"</small></strong><strong>{move ||money(shop.total())}</strong></div></div></Show>}
}
#[component]
fn CheckoutAddress(shop: Shop) -> impl IntoView {
    let d = shop.details;
    view! {<div class="checkout-address"><Icon name="pin"/><div><strong>{move ||d.name.get()}</strong><span>{move ||d.organization.get()}</span><span>{move ||d.street.get()}</span><span>{move ||format!("{} {}",d.postal.get(),d.city.get())}</span><span>{move ||d.notes.get()}</span></div></div><div class="checkout-address"><Icon name="clock"/><div><strong>{move ||shop.delivery_label()}</strong><span>"Požadovaný termín, čeká na potvrzení"</span></div></div><div class="checkout-address"><Icon name="person"/><div><strong>{move ||d.email.get()}</strong><span>{move ||d.phone.get()}</span></div></div>}
}
#[component]
fn CheckoutItems(shop: Shop) -> impl IntoView {
    view! {<div class="checkout-review-items">{move ||shop.products.get().into_iter().filter(|p|shop.count(p.id)>0).map(|p|view! {<div><span><b>{shop.count(p.id)}"×"</b>{p.name}</span><strong>{money(p.price_cents*i64::from(shop.count(p.id)))}</strong></div>}).collect_view()}</div>}
}
#[component]
fn CheckoutStepper(shop: Shop) -> impl IntoView {
    view! {<ol class="checkout-stepper" aria-label="Kroky objednávky" hidden=move ||shop.step.get()==3>{[(0,"Košík"),(1,"Doručení"),(2,"Potvrzení")].into_iter().map(move |(step,label)|view! {<li class:current=move ||shop.step.get()==step class:complete={move ||shop.step.get()>step}><button type="button" disabled={move ||step>=shop.step.get() ||shop.submitting.get()} aria-current=move ||(shop.step.get()==step).then_some("step") on:click=move |_|shop.go(step)><span class="step-dot">{step+1}</span><span>{label}</span></button></li>}).collect_view()}</ol>}
}
#[component]
fn CartDialog(shop: Shop) -> impl IntoView {
    let d = shop.details;
    view! {
        <dialog node_ref=shop.cart_dialog class="cart-dialog checkout-dialog" aria-labelledby="cart-title">
            <div class="dialog-heading"><div><p class="eyebrow">"VAŠE DOBRÁ PAUZA"</p><h2 id="cart-title" node_ref=shop.cart_title tabindex="-1">{move ||match shop.step.get() {1=>"Kam to přivezeme?",2=>"Už jen poslední krok.",3=>match shop.confirmation.get().map(|o|o.status).as_deref() {Some("awaiting_payment")=>"Čekáme na zaplacení.",Some("cancelled")=>"Objednávka byla zrušena.",Some("expired")=>"Platnost objednávky vypršela.",_=>"Objednávku jsme přijali."},_=>"Váš košík"}}</h2></div><button class="icon-button" aria-label="Zavřít košík" on:click=move |_|close_dialog(shop.cart_dialog)><Icon name="close"/></button></div>
            <Show when={move ||shop.step.get()==3 ||catalog::quantity(&shop.cart.get())>0} fallback=move ||view! {<div class="empty-cart"><span class="empty-cart-icon"><Icon name="bag"/></span><h3>"Vaše pauza je zatím prázdná."</h3><p>"Přidejte si něco dobrého z naší nabídky."</p><Show when=move ||!shop.order_error.get().is_empty()><div class="api-message error" role="alert">{move ||shop.order_error.get()}</div></Show><button class="button primary" on:click=move |_|close_dialog(shop.cart_dialog)>"Jdu si vybrat"<Icon/></button></div>}>
                <CheckoutStepper shop/>
                <div class="checkout-body" node_ref=shop.cart_content>
                    <Show when=move ||!shop.notice.get().is_empty()><div class="api-message" role="status">{move ||shop.notice.get()}</div></Show>
                    <Show when=move ||!shop.error.get().is_empty()><div class="api-message error" role="alert"><p>{move ||shop.error.get()}</p><button class="button secondary" on:click=move |_|shop.refresh()>"Obnovit nabídku"</button></div></Show>
                    <section data-checkout-step="cart" hidden=move ||shop.step.get()!=0 aria-label="Obsah košíku">
                        <Show when=move ||shop.settings.get().free_delivery_from_cents.is_some()><div class="free-delivery"><Icon name="truck"/><p>{move ||{let limit=shop.settings.get().free_delivery_from_cents.unwrap_or(0);if shop.subtotal()>=limit {"Doručení máte od nás. Dobrou chuť!".into()} else {format!("Ještě {} a doručení máte zdarma.",money(limit-shop.subtotal()))}}}</p><progress max=move ||shop.settings.get().free_delivery_from_cents.unwrap_or(1).max(1) value=move ||shop.subtotal() aria-label="Do dopravy zdarma"/></div></Show>
                        <div class="cart-items"><For each={move ||shop.products.get().into_iter().filter(|p|shop.count(p.id)>0).collect::<Vec<_>>()} key=|p|(p.id,p.version,p.price_cents) children=move |p|view! {<article class="cart-item"><div class=format!("cart-item-photo {}",photo_class(&p)) style=photo_style(&p) role="img" aria-label=p.name.clone()></div><div class="cart-item-info"><h3>{p.name}</h3><p>{p.weight}" · "{money(p.price_cents)}" / ks"</p><div class="cart-item-actions"><QuantityControl id=p.id shop/><strong>{move ||money(p.price_cents*i64::from(shop.count(p.id)))}</strong></div></div></article>}/></div>
                        <CartTotals shop/><button class="button primary full-width" disabled=move ||!shop.ready.get() on:click=move |_|shop.go(1)>"Pokračovat k doručení"<Icon/></button><p class="checkout-reassurance"><Icon name="check"/>"Bez registrace. Za pár chvil hotovo."</p>
                    </section>
                    <section data-checkout-step="delivery" hidden=move ||shop.step.get()!=1 aria-label="Doručovací údaje">
                        <div class="checkout-guest"><span class="checkout-guest-icon"><Icon name="person"/></span><div><strong>{move ||if shop.user.get().is_some() {"Jste přihlášeni"} else {"Nakoupíte i bez účtu"}}</strong><span>"Povinný je jen e-mail a doručovací adresa."</span></div></div>
                        <Show when=move ||shop.user.get().is_none()><button class="text-button optional-login" type="button" on:click=move |_|shop.open_account()>"Mám účet, přihlásit se"</button></Show>
                        <form class="checkout-form" on:submit=move |ev|{ev.prevent_default();if shop.delivers_to_postal() {shop.go(2);}}>
                            <fieldset class="checkout-fieldset"><legend>"Kam poslat potvrzení"</legend><label class="form-field">"E-mail"<input type="email" name="email" autocomplete="email" required maxlength="254" prop:value=move ||d.email.get() on:input=move |ev|d.email.set(event_target_value(&ev))/></label></fieldset>
                            <fieldset class="checkout-fieldset"><legend>"Doručovací adresa"</legend><label class="form-field">"Ulice a číslo domu"<input name="street-address" autocomplete="street-address" pattern=r".*\S.*" required maxlength="300" prop:value=move ||d.street.get() on:input=move |ev|d.street.set(event_target_value(&ev))/></label><div class="form-row address-row"><label class="form-field">"Město"<input name="city" autocomplete="address-level2" pattern=r".*\S.*" required maxlength="120" prop:value=move ||d.city.get() on:input=move |ev|d.city.set(event_target_value(&ev))/></label><label class="form-field">"PSČ"<input name="postal-code" aria-describedby="checkout-postal-status" autocomplete="postal-code" inputmode="numeric" pattern="[0-9]{5}" title="Zadejte pět číslic, například 110 00" required prop:value=move ||d.postal.get() on:input=move |ev|update_postal_input(&ev,d.postal)/></label></div><PostalAvailability shop id="checkout-postal-status"/><label class="form-field">"Pokyny pro doručení (nepovinné)"<textarea name="notes" rows="3" maxlength="2000" placeholder="Např. 3. patro, zvonek Novákovi, předat na recepci" prop:value=move ||d.notes.get() on:input=move |ev|d.notes.set(event_target_value(&ev))></textarea></label>
                            <details class="optional-details"><summary>"Přidat jméno, telefon nebo firmu"</summary><label class="form-field">"Jméno (nepovinné)"<input name="name" autocomplete="name" maxlength="120" prop:value=move ||d.name.get() on:input=move |ev|d.name.set(event_target_value(&ev))/></label><label class="form-field">"Telefon pro kurýra (nepovinné)"<input type="tel" name="tel" autocomplete="tel" maxlength="40" prop:value=move ||d.phone.get() on:input=move |ev|d.phone.set(event_target_value(&ev))/></label><label class="form-field">"Firma (nepovinné)"<input name="organization" autocomplete="organization" maxlength="200" prop:value=move ||d.organization.get() on:input=move |ev|d.organization.set(event_target_value(&ev))/></label></details></fieldset>
                            <fieldset class="checkout-fieldset"><legend>"Kdy se vám to hodí?"</legend><DeliveryFields shop prefix="checkout"/></fieldset>
                            <div class="checkout-next-total"><span>"Celkem včetně doručení"</span><strong>{move ||money(shop.total())}</strong></div><button class="button primary full-width" type="submit" disabled=move ||!shop.delivers_to_postal() ||shop.day.get().is_empty() ||shop.slot.get().is_empty()>"Zkontrolovat objednávku"<Icon/></button><button class="text-button checkout-back" type="button" on:click=move |_|shop.go(0)><Icon name="back"/>"Zpět do košíku"</button>
                        </form>
                    </section>
                    <section data-checkout-step="payment" hidden=move ||shop.step.get()!=2 aria-label="Platba a kontrola objednávky">
                        <div class="checkout-review"><div class="checkout-section-heading"><h3>"Vaše dobrá pauza"</h3><button class="text-button" disabled=move ||shop.submitting.get() type="button" on:click=move |_|shop.go(1)>"Upravit doručení"</button></div><CheckoutAddress shop/><PostalAvailability shop id="review-postal-status"/><CheckoutItems shop/></div>
                        <fieldset class="payment-options" disabled=move ||shop.submitting.get()><legend>"Jak zaplatíte?"</legend><Show when=move ||shop.settings.get().online_payment_enabled><label class="payment-option" class:selected=move ||shop.payment.get()=="card"><input type="radio" name="payment-method" value="card" prop:checked=move ||shop.payment.get()=="card" on:change=move |_|shop.payment.set("card".into())/><Icon name="card"/><span><strong>"Online kartou"</strong><small>"Bezpečně na platební bráně Teya"</small></span></label></Show><label class="payment-option" class:selected=move ||shop.payment.get()=="card_on_delivery"><input type="radio" name="payment-method" value="card_on_delivery" prop:checked=move ||shop.payment.get()=="card_on_delivery" on:change=move |_|shop.payment.set("card_on_delivery".into())/><Icon name="card"/><span><strong>"Kartou při doručení"</strong><small>"Přiložíte kartu nebo telefon k terminálu"</small></span></label><label class="payment-option" class:selected=move ||shop.payment.get()=="cash_on_delivery"><input type="radio" name="payment-method" value="cash_on_delivery" prop:checked=move ||shop.payment.get()=="cash_on_delivery" on:change=move |_|shop.payment.set("cash_on_delivery".into())/><Icon name="wallet"/><span><strong>"Hotově při doručení"</strong><small>"Zaplatíte kurýrovi při převzetí"</small></span></label></fieldset>
                        <CartTotals shop/>
                        <Show when=move ||!shop.order_error.get().is_empty()><div class="api-message error" role="alert">{move ||shop.order_error.get()}</div></Show>
                        <p class="field-hint checkout-order-info">{move ||if shop.payment.get()=="card" {"Odesláním vytvoříte objednávku s povinností platby a přejdete na platební bránu Teya. Objednávku přijmeme až po ověření platby a potvrdíme e-mailem. Požadovaný termín ještě potvrdí obsluha."} else {"Odesláním vytvoříte objednávku s povinností platby při doručení. Přijetí vám potvrdíme e-mailem. Požadovaný termín ještě potvrdí obsluha."}}</p>
                        <button class="button primary full-width payment-submit" type="button" disabled=move ||shop.submitting.get() ||!shop.delivers_to_postal() ||shop.day.get().is_empty() ||shop.slot.get().is_empty() ||(shop.payment.get()=="card" && !shop.settings.get().online_payment_enabled) on:click=move |_|shop.submit()><Icon name="check"/>{move ||if shop.submitting.get() {"Odesíláme objednávku…".into()} else {format!("Objednat s povinností platby · {}",money(shop.total()))}}</button><button class="text-button checkout-back" type="button" disabled=move ||shop.submitting.get() on:click=move |_|shop.go(1)><Icon name="back"/>"Zpět k doručení"</button>
                    </section>
                    <section data-checkout-step="confirmation" hidden=move ||shop.step.get()!=3 class="checkout-confirmation" aria-label="Potvrzení objednávky">
                        {move ||shop.confirmation.get().map(|order|view! {<div class="confirmation-intro"><span class="confirmation-mark"><Icon name="check"/></span><p class="eyebrow">"DĚKUJEME ZA OBJEDNÁVKU"</p><h3>{if order.status=="expired" {"Lhůta pro zaplacení skončila."} else {"Dobrá pauza je na dohled."}}</h3><p>"Číslo objednávky "<strong>{order.number.clone()}</strong><br/>{match order.status.as_str() {"awaiting_payment"=>"Objednávku přijmeme po ověření platby.","cancelled"=>"Objednávka je zrušena. Podrobnosti najdete u platby.","expired"=>"Objednávka nebyla včas zaplacena a je uzavřena.",_=>"Přijetí vám potvrdíme e-mailem."}}</p></div><OrderSummary order/>})}
                        <PaymentProgress shop/><p class="confirmation-note">{move ||shop.confirmation.get().map(|o|if o.status=="expired" {"Tato objednávka už nebude vyřízena. Pokud jste platbu odeslali, kontaktujte obsluhu s číslem objednávky."} else if o.status=="cancelled" {"Tato objednávka byla zrušena."} else if o.payment_method=="card" {if o.payment_status=="paid" {"Platba je ověřená. Požadovaný termín ještě potvrdíme."} else {"Objednávka je uložená a čeká na ověření platby. Nevytvářejte ji znovu."}} else {"Platíte až při doručení. Požadovaný termín ještě potvrdíme."})}</p><Show when=move ||shop.user.get().is_none()><button class="button secondary full-width" on:click=move |_|shop.open_account()>"Příště rychleji? Vytvořit účet e-mailem"</button><p class="field-hint">"Účet ukládá objednávky vytvořené po přihlášení. Tato objednávka vám zůstane v e-mailu."</p></Show><button class="button primary full-width" on:click=move |_|{shop.confirmation.set(None);shop.go(0);close_dialog(shop.cart_dialog);}>"Zpátky na svačiny"<Icon/></button>
                    </section>
                </div>
            </Show>
        </dialog>
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
    let payment = match order.payment_method.as_str() {
        "card" if order.payment_status == "paid" => "Online kartou · zaplaceno",
        "card" => "Online kartou · dosud neověřeno",
        "cash_on_delivery" => "Hotově při doručení",
        _ => "Kartou při doručení",
    };
    view! {
        <div class="confirmation-details"><div class="order-heading"><h4>{format!("Objednávka {}",order.number)}</h4><span class="order-status">{catalog::status_label(&order.status)}</span></div><div class="checkout-address"><Icon name="pin"/><div><strong>{order.street}</strong><span>{format!("{} {}",order.postal_code,order.city)}</span><span>{order.notes.unwrap_or_default()}</span></div></div><div class="checkout-address"><Icon name="clock"/><div><strong>{format!("{} · {}",order.delivery_date,order.delivery_slot)}</strong><span>{if order.status=="new" {"Požadovaný termín, čeká na potvrzení"} else {"Termín objednávky"}}</span></div></div><div class="checkout-address"><Icon name="card"/><div><strong>{payment}</strong><span>{order.email}</span></div></div><div class="checkout-review-items">{order.items.into_iter().map(|item|view! {<div><span><b>{item.quantity}"×"</b>{item.name}</span><strong>{money(item.total_cents)}</strong></div>}).collect_view()}</div><div class="cart-totals"><div><span>"Svačiny"</span><span>{money(order.subtotal_cents)}</span></div><div><span>"Doručení"</span><span>{money(order.delivery_fee_cents)}</span></div><div class="grand-total"><strong>"Celkem"</strong><strong>{money(order.total_cents)}</strong></div></div></div>
    }
}
#[component]
fn AccountOrder(order: Order, shop: Shop) -> impl IntoView {
    let address = order.clone();
    let reorder = order.clone();
    view! {
        <article class="account-order"><details><summary><span><strong>{order.number.clone()}</strong><small>{order.delivery_date.clone()}</small></span><span class="order-status">{catalog::status_label(&order.status)}</span><strong>{money(order.total_cents)}</strong></summary><OrderSummary order=order.clone()/></details><div class="order-actions"><button class="button secondary" disabled=move ||!shop.ready.get() on:click=move |_|{
            let products=shop.products.get_untracked();let mut omitted=false;
            shop.cart.update(|cart|{for item in &reorder.items {if let Some(product)=products.iter().find(|p|p.id==item.product_id && p.active && p.stock!=Some(0)) {let max=product.stock.unwrap_or(99).clamp(0,99) as u16;let existing=cart.entry(item.product_id).or_default();let requested=existing.saturating_add(item.quantity);*existing=requested.min(max);if requested>max {omitted=true;}} else {omitted=true;}}});
            shop.details.use_address(&reorder);shop.confirmation.set(None);
            shop.notice.set(if omitted {"Do košíku jsme přidali dostupné položky. Některé svačiny nebo počty už nejsou dostupné. Platí aktuální ceny.".into()} else {"Svačiny jsme přidali do košíku za aktuální ceny. Zkontrolujte nový termín doručení.".into()});
            close_dialog(shop.account_dialog);shop.open_cart();
        }>"Přidat znovu do košíku"</button><button class="text-button" on:click=move |_|{shop.details.use_address(&address);shop.auth_message.set("Doručovací adresu jsme předvyplnili pro další objednávku.".into());}>"Použít tuto adresu"</button></div></article>
    }
}
#[component]
fn AccountDialog(shop: Shop) -> impl IntoView {
    let request_link = move || {
        if shop.auth_busy.get_untracked() {
            return;
        }
        shop.auth_busy.set(true);
        shop.account_error.set(String::new());
        shop.auth_message.set(String::new());
        let email = shop.details.email.get_untracked();
        spawn_local(async move {
            match api::post::<serde_json::Value>("/api/auth/request",&serde_json::json!({"email":email})).await {
                Ok(_) => shop.auth_message.set("Pokud je adresa správná, za chvíli v e-mailu najdete přihlašovací odkaz. Platí 15 minut. Zkontrolujte také spam.".into()),
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
            <Show when=move ||shop.user.get().is_none() &&shop.login_token.get().is_none()><div class="account-intro"><Icon name="person"/><p>"Stačí váš e-mail. Pošleme odkaz pro přihlášení, při prvním použití rovnou vytvoříme účet. Žádné heslo si nemusíte pamatovat."</p></div><form on:submit=move |ev|{ev.prevent_default();request_link();}><label class="form-field">"Váš e-mail"<input type="email" autocomplete="email" name="login-email" required maxlength="254" prop:value=move ||shop.details.email.get() on:input=move |ev|shop.details.email.set(event_target_value(&ev))/></label><button class="button primary full-width" type="submit" disabled=move ||shop.auth_busy.get()>{move ||if shop.auth_busy.get() {"Posíláme odkaz…"} else {"Poslat přihlašovací odkaz"}}</button></form><p class="field-hint">"S účtem uvidíte své další objednávky, snadno je zopakujete a použijete minulou adresu. Nákup bez účtu je vždy možný."</p></Show>
            <Show when=move ||shop.user.get().is_some()><div class="account-identity"><strong>{move ||shop.user.get().map(|u|u.email).unwrap_or_default()}</strong><button class="text-button" disabled=move ||shop.auth_busy.get() on:click=move |_|{
                shop.auth_busy.set(true);
                spawn_local(async move {match api::post::<serde_json::Value>("/api/auth/logout",&serde_json::json!({})).await {Ok(_) => {shop.user.set(None);shop.orders.set(Vec::new());shop.auth_message.set("Jste odhlášeni.".into());},Err(e)=>shop.account_error.set(e.message)}shop.auth_busy.set(false);});
            }>"Odhlásit"</button></div><Show when=move ||shop.user.get().is_some_and(|u|u.is_admin)><a class="button secondary full-width" href="/admin/">"Otevřít administraci"</a></Show><div class="account-orders-heading"><h3>"Moje objednávky"</h3><button class="text-button" disabled=move ||shop.account_loading.get() on:click=move |_|shop.load_orders()>"Obnovit stav"</button></div><Show when=move ||shop.account_loading.get()><p class="shop-status" role="status">"Načítáme objednávky…"</p></Show><Show when=move ||!shop.account_loading.get() &&shop.orders.get().is_empty()><p class="account-empty">"Vaše první objednávka na vás teprve čeká. Objednávky bez přihlášení najdete ve svém e-mailu."</p></Show><div class="account-orders"><For each=move ||shop.orders.get() key=|o|(o.id.clone(),o.status.clone()) children=move |order|view! {<AccountOrder order shop/>}/></div></Show>
        </dialog>
    }
}
