//! API data and cart calculations. All amounts use integer Czech haléře.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Product {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub category: String,
    pub tag: String,
    pub weight: String,
    pub price_cents: i64,
    pub vegan: bool,
    pub ingredients: String,
    pub allergens: String,
    pub image: String,
    pub active: bool,
    pub stock: Option<i32>,
    #[serde(default)]
    pub minimum_quantity: u16,
    pub version: i64,
}

impl Product {
    pub fn minimum(&self) -> u16 {
        self.minimum_quantity.max(1)
    }

    pub fn maximum(&self) -> u16 {
        self.stock.unwrap_or(99).clamp(0, 99) as u16
    }

    pub fn orderable_quantity(&self, requested: u16) -> Option<u16> {
        let (min, max) = (self.minimum(), self.maximum());
        (self.active && min <= max).then(|| requested.clamp(min, max))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Settings {
    pub delivery_fee_cents: i64,
    pub free_delivery_from_cents: Option<i64>,
    pub delivery_postal_codes: Vec<String>,
    #[serde(default)]
    pub online_payment_enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DeliveryOptions {
    pub dates: Vec<DeliveryDate>,
    pub slots: Vec<String>,
    #[serde(default)]
    pub postal_slots: std::collections::BTreeMap<String, Vec<String>>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct DeliveryDate {
    pub value: String,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub is_admin: bool,
}
#[derive(Clone, Debug, Deserialize)]
pub struct UserResponse {
    pub user: User,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Order {
    pub id: String,
    pub number: String,
    pub status: String,
    pub payment_method: String,
    #[serde(default)]
    pub payment_status: String,
    #[serde(default)]
    pub payment_expires_at: Option<String>,
    pub subtotal_cents: i64,
    pub delivery_fee_cents: i64,
    pub total_cents: i64,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub street: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub postal_code: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub delivery_date: String,
    #[serde(default)]
    pub delivery_slot: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub items: Vec<OrderItem>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrderItem {
    pub product_id: i64,
    pub name: String,
    pub quantity: u16,
    pub unit_price_cents: i64,
    pub total_cents: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaymentResponse {
    pub status: String,
    pub payment_status: String,
    pub checkout_url: Option<String>,
    pub payment_state: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PaymentRecovery {
    pub order: Order,
    pub idempotency_key: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct RequestedItem {
    pub product_id: i64,
    pub quantity: u16,
}
#[derive(Clone, Debug, Serialize)]
pub struct OrderRequest {
    pub idempotency_key: String,
    pub email: String,
    pub name: String,
    pub phone: String,
    pub organization: String,
    pub street: String,
    pub city: String,
    pub postal_code: String,
    pub notes: String,
    pub delivery_date: String,
    pub delivery_slot: String,
    pub payment_method: String,
    pub items: Vec<RequestedItem>,
    pub expected_total_cents: i64,
}

pub type Cart = BTreeMap<i64, u16>;

pub fn normalize_postal_code(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{feff}')
        .collect()
}
pub fn valid_postal_code(value: &str) -> bool {
    value.len() == 5 && value.bytes().all(|c| c.is_ascii_digit())
}
pub fn delivery_available(value: &str, settings: &Settings) -> bool {
    let normalized = normalize_postal_code(value);
    valid_postal_code(&normalized) && settings.delivery_postal_codes.contains(&normalized)
}

pub fn subtotal(cart: &Cart, products: &[Product]) -> i64 {
    products
        .iter()
        .map(|p| p.price_cents * i64::from(cart.get(&p.id).copied().unwrap_or(0)))
        .sum()
}
pub fn delivery_fee(amount: i64, settings: &Settings) -> i64 {
    if amount == 0
        || settings
            .free_delivery_from_cents
            .is_some_and(|limit| amount >= limit)
    {
        0
    } else {
        settings.delivery_fee_cents
    }
}
pub fn quantity(cart: &Cart) -> u32 {
    cart.values().map(|n| u32::from(*n)).sum()
}
pub fn parse_cart(value: &str) -> Cart {
    serde_json::from_str::<Cart>(value)
        .unwrap_or_default()
        .into_iter()
        .filter(|(id, n)| *id > 0 && *n > 0)
        .map(|(id, n)| (id, n.min(99)))
        .collect()
}
pub fn reconcile(cart: &mut Cart, products: &[Product]) -> bool {
    let previous = cart.clone();
    cart.retain(|id, count| {
        if let Some(quantity) = products
            .iter()
            .find(|p| p.id == *id)
            .and_then(|p| p.orderable_quantity(*count))
        {
            *count = quantity;
            true
        } else {
            false
        }
    });
    *cart != previous
}
pub fn money(cents: i64) -> String {
    if cents % 100 == 0 {
        format!("{} Kč", cents / 100)
    } else {
        format!("{},{:02} Kč", cents / 100, cents.abs() % 100)
    }
}
pub fn status_label(value: &str) -> &'static str {
    match value {
        "awaiting_payment" => "Čeká na zaplacení",
        "new" => "Přijato",
        "confirmed" => "Potvrzeno",
        "out_for_delivery" => "Na cestě",
        "delivered" => "Doručeno",
        "cancelled" => "Zrušeno",
        "expired" => "Vypršela",
        _ => "Zpracovává se",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn product(id: i64, cents: i64, stock: Option<i32>) -> Product {
        Product {
            id,
            name: "Svačina".into(),
            description: String::new(),
            category: String::new(),
            tag: String::new(),
            weight: String::new(),
            price_cents: cents,
            vegan: false,
            ingredients: String::new(),
            allergens: String::new(),
            image: String::new(),
            active: true,
            stock,
            minimum_quantity: 0,
            version: 1,
        }
    }
    #[test]
    fn cart_uses_stable_ids_and_exact_cents() {
        let products = vec![product(90, 11950, None), product(7, 6500, None)];
        let cart = Cart::from([(7, 2), (90, 1)]);
        assert_eq!(subtotal(&cart, &products), 24950);
        assert_eq!(quantity(&cart), 3);
        assert_eq!(money(24950), "249,50 Kč");
    }
    #[test]
    fn delivery_respects_optional_threshold_and_exact_boundary() {
        let mut settings = Settings {
            delivery_fee_cents: 4900,
            free_delivery_from_cents: Some(35000),
            delivery_postal_codes: vec!["11000".into()],
            online_payment_enabled: false,
        };
        assert_eq!(delivery_fee(34999, &settings), 4900);
        assert_eq!(delivery_fee(35000, &settings), 0);
        settings.free_delivery_from_cents = None;
        assert_eq!(delivery_fee(99000, &settings), 4900);
        assert_eq!(delivery_fee(0, &settings), 0);
    }
    #[test]
    fn postal_codes_ignore_unicode_whitespace_and_require_five_ascii_digits() {
        for value in [
            "11000",
            " 110 00 ",
            "\t11\n0\r00\t",
            "1\u{a0}1\u{2003}0\u{202f}0\u{0085}0\u{feff}",
        ] {
            let normalized = normalize_postal_code(value);
            assert_eq!(normalized, "11000");
            assert!(valid_postal_code(&normalized));
        }
        for value in ["", "1100", "110000", "110-00", "11x00", "１１０００"] {
            assert!(!valid_postal_code(&normalize_postal_code(value)));
        }
    }
    #[test]
    fn only_configured_postal_codes_allow_delivery() {
        let mut settings = Settings::default();
        assert!(!delivery_available("11000", &settings));
        settings.delivery_postal_codes = vec!["11000".into(), "60200".into()];
        assert!(delivery_available("110\u{a0}00", &settings));
        assert!(delivery_available("602 00", &settings));
        assert!(!delivery_available("12000", &settings));
        assert!(!delivery_available("11x00", &settings));
    }
    #[test]
    fn minimum_applies_to_first_add_reorder_and_restored_carts() {
        let mut snack = product(18, 100, Some(8));
        assert_eq!(snack.orderable_quantity(1), Some(1));
        snack.minimum_quantity = 5;
        assert_eq!(snack.orderable_quantity(1), Some(5));
        assert_eq!(snack.orderable_quantity(6), Some(6));
        assert_eq!(snack.orderable_quantity(10), Some(8));
        let mut cart = Cart::from([(18, 2)]);
        assert!(reconcile(&mut cart, std::slice::from_ref(&snack)));
        assert_eq!(cart[&18], 5);
        snack.stock = Some(4);
        assert_eq!(snack.orderable_quantity(1), None);
        assert!(reconcile(&mut cart, &[snack]));
        assert!(cart.is_empty());
    }

    #[test]
    fn persisted_cart_recovers_safely_and_reconciles_stock() {
        assert!(parse_cart("broken").is_empty());
        let mut cart = parse_cart(r#"{"18":200,"7":2,"0":4,"22":1}"#);
        assert_eq!(cart.get(&18), Some(&99));
        assert!(!cart.contains_key(&0));
        assert!(reconcile(
            &mut cart,
            &[product(18, 100, Some(3)), product(7, 100, Some(0))]
        ));
        assert_eq!(cart, Cart::from([(18, 3)]));
    }
}
