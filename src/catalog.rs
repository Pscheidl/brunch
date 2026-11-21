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
    pub minimum_quantity: u32,
    #[serde(default)]
    pub banquet: bool,
    #[serde(default)]
    pub variants: Vec<String>,
    pub version: i64,
}

impl Product {
    pub fn minimum(&self) -> u32 {
        self.minimum_quantity.max(1)
    }

    pub fn maximum(&self) -> u32 {
        let limit = if self.banquet { i32::MAX } else { 99 };
        self.stock.unwrap_or(limit).clamp(0, limit) as u32
    }

    pub fn orderable_quantity(&self, requested: u32) -> Option<u32> {
        let (min, max) = (self.minimum(), self.maximum());
        (self.active && min <= max).then(|| requested.clamp(min, max))
    }

    pub fn total_price_cents(&self, quantity: u32) -> i64 {
        self.price_cents.saturating_mul(i64::from(quantity))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Settings {
    #[serde(default = "default_banquet_minimum")]
    pub banquet_minimum_quantity: u32,
    #[serde(default)]
    pub pickup: PickupSettings,
    pub delivery_fee_cents: i64,
    pub free_delivery_from_cents: Option<i64>,
    pub delivery_postal_codes: Vec<String>,
    #[serde(default)]
    pub online_payment_enabled: bool,
    #[serde(default)]
    pub hero_image: String,
    #[serde(default)]
    pub hero_image_alt: String,
    #[serde(default)]
    pub public_contact_email: String,
    #[serde(default)]
    pub public_contact_phone: String,
    #[serde(default)]
    pub seller_name: String,
    #[serde(default)]
    pub seller_address: String,
    #[serde(default)]
    pub seller_ico: String,
    #[serde(default)]
    pub delivery_area_label: String,
}

fn default_banquet_minimum() -> u32 {
    30
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            banquet_minimum_quantity: default_banquet_minimum(),
            pickup: PickupSettings::default(),
            delivery_fee_cents: 0,
            free_delivery_from_cents: None,
            delivery_postal_codes: Vec::new(),
            online_payment_enabled: false,
            hero_image: String::new(),
            hero_image_alt: String::new(),
            public_contact_email: String::new(),
            public_contact_phone: String::new(),
            seller_name: String::new(),
            seller_address: String::new(),
            seller_ico: String::new(),
            delivery_area_label: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct PickupSettings {
    pub enabled: bool,
    pub name: String,
    pub street: String,
    pub city: String,
    pub postal_code: String,
    pub instructions: String,
    pub parking_free: bool,
    pub slots: Vec<String>,
    pub version: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PickupLocation {
    pub name: String,
    pub street: String,
    pub city: String,
    pub postal_code: String,
    pub instructions: String,
    pub parking_free: bool,
}

pub const DEFAULT_HERO_IMAGE: &str = "images/brunch-spread.webp";
pub const DEFAULT_HERO_ALT: &str =
    "Kváskový sendvič, jogurt s granolou a hummus se zeleninou na prostřeném stole";

#[derive(Clone, Debug, Deserialize)]
pub struct Terms {
    pub version: i64,
    pub title: String,
    pub content: String,
    pub updated_at: String,
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
    #[serde(default)]
    pub fulfillment_method: String,
    #[serde(default)]
    pub pickup_location: Option<PickupLocation>,
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
impl Order {
    pub fn is_pickup(&self) -> bool {
        self.fulfillment_method == "pickup"
    }
    pub fn status_label(&self) -> &'static str {
        match (self.is_pickup(), self.status.as_str()) {
            (true, "out_for_delivery") => "Připraveno k vyzvednutí",
            (true, "delivered") => "Vyzvednuto",
            _ => status_label(&self.status),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrderItem {
    #[serde(default)]
    pub variant: Option<String>,
    pub product_id: i64,
    pub name: String,
    pub quantity: u32,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub product_id: i64,
    pub quantity: u32,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BillingInput {
    pub name: String,
    pub address: String,
    pub company_id: String,
    pub tax_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct OrderRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fulfillment_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pickup_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing: Option<BillingInput>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terms_version: Option<i64>,
}

impl OrderRequest {
    /// Recover an unresolved attempt when only the displayed terms changed.
    /// Every other field must match, including the cart, price, address and date.
    pub fn recover_terms_for_retry(&mut self, signature: &str) -> bool {
        let Ok(serde_json::Value::Object(mut previous)) = serde_json::from_str(signature) else {
            return false;
        };
        let previous_terms = match previous.remove("terms_version") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => match value.as_i64().filter(|version| *version > 0) {
                Some(version) => Some(version),
                None => return false,
            },
        };
        let Ok(serde_json::Value::Object(mut current)) = serde_json::to_value(&*self) else {
            return false;
        };
        current.remove("terms_version");
        if previous != current {
            return false;
        }
        self.terms_version = previous_terms;
        true
    }
}

pub type Cart = BTreeMap<i64, u32>;

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
        .map(|p| p.total_price_cents(cart.get(&p.id).copied().unwrap_or(0)))
        .fold(0, i64::saturating_add)
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
pub fn quantity(cart: &Cart) -> u64 {
    cart.values().map(|n| u64::from(*n)).sum()
}
pub fn banquet_quantity(cart: &Cart, products: &[Product]) -> u64 {
    products
        .iter()
        .filter(|product| product.banquet)
        .map(|product| u64::from(cart.get(&product.id).copied().unwrap_or(0)))
        .sum()
}
pub fn banquet_missing(cart: &Cart, products: &[Product], minimum: u32) -> u64 {
    let quantity = banquet_quantity(cart, products);
    if quantity == 0 {
        0
    } else {
        u64::from(minimum).saturating_sub(quantity)
    }
}
pub fn parse_cart(value: &str) -> Cart {
    serde_json::from_str::<Cart>(value)
        .unwrap_or_default()
        .into_iter()
        .filter(|(id, n)| *id > 0 && *n > 0)
        .map(|(id, n)| (id, n.min(i32::MAX as u32)))
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
        format!("{},{:02} Kč", cents / 100, cents.unsigned_abs() % 100)
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
    fn order_request() -> OrderRequest {
        OrderRequest {
            fulfillment_method: None,
            pickup_version: None,
            team_order_id: None,
            team_revision: None,
            billing: None,
            idempotency_key: String::new(),
            email: "test@example.test".into(),
            name: "Jana Testovací".into(),
            phone: "+420 777 123 456".into(),
            organization: String::new(),
            street: "Testovací 12".into(),
            city: "Praha".into(),
            postal_code: "11000".into(),
            notes: String::new(),
            delivery_date: "2026-11-23".into(),
            delivery_slot: "09:00–10:00".into(),
            payment_method: "cash_on_delivery".into(),
            items: vec![RequestedItem {
                variant: None,
                product_id: 1,
                quantity: 1,
            }],
            expected_total_cents: 12000,
            terms_version: Some(2),
        }
    }

    #[test]
    fn terms_only_changes_recover_the_original_request_including_legacy_payloads() {
        let mut original = order_request();
        original.terms_version = Some(1);
        let mut retry = order_request();
        assert!(retry.recover_terms_for_retry(&serde_json::to_string(&original).unwrap()));
        assert_eq!(retry.terms_version, Some(1));
        original.terms_version = None;
        let signature = serde_json::to_string(&original).unwrap();
        let mut retry = order_request();
        assert!(retry.recover_terms_for_retry(&signature));
        assert_eq!(serde_json::to_string(&retry).unwrap(), signature);
        assert!(!signature.contains("terms_version"));
    }

    #[test]
    fn other_order_changes_do_not_recover_an_unrelated_attempt() {
        let original = serde_json::to_string(&order_request()).unwrap();
        for change in ["cart", "price", "contact", "date"] {
            let mut current = order_request();
            match change {
                "cart" => current.items[0].quantity = 2,
                "price" => current.expected_total_cents = 13000,
                "contact" => current.phone = "+420 777 000 000".into(),
                "date" => current.delivery_date = "2026-11-24".into(),
                _ => unreachable!(),
            }
            assert!(!current.recover_terms_for_retry(&original), "{change}");
            assert_eq!(current.terms_version, Some(2));
        }
        assert!(!order_request().recover_terms_for_retry("broken"));
    }

    fn product(id: i64, cents: i64, stock: Option<i32>) -> Product {
        Product {
            variants: Vec::new(),
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
            banquet: false,
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
    fn banquet_minimum_combines_only_banquet_products() {
        let mut products = vec![
            product(1, 100, None),
            product(2, 100, None),
            product(3, 100, None),
            product(4, 100, None),
        ];
        for item in &mut products[..3] {
            item.banquet = true;
            item.minimum_quantity = 10;
        }
        let mut cart = Cart::from([(1, 10), (2, 10), (4, 99)]);
        assert_eq!(banquet_quantity(&cart, &products), 20);
        assert_eq!(banquet_missing(&cart, &products, 30), 10);
        cart.insert(3, 10);
        assert_eq!(banquet_quantity(&cart, &products), 30);
        assert_eq!(banquet_missing(&cart, &products, 30), 0);
        assert_eq!(banquet_missing(&cart, &products, 31), 1);
        cart.insert(3, 11);
        assert_eq!(banquet_missing(&cart, &products, 30), 0);
        assert_eq!(banquet_missing(&Cart::from([(4, 1)]), &products, 30), 0);
        assert_eq!(banquet_missing(&Cart::new(), &products, 30), 0);
    }
    #[test]
    fn old_api_payload_defaults_to_non_banquet_and_thirty_piece_minimum() {
        let mut value = serde_json::to_value(product(1, 100, None)).unwrap();
        value.as_object_mut().unwrap().remove("banquet");
        assert!(!serde_json::from_value::<Product>(value).unwrap().banquet);
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "delivery_fee_cents":4900,"free_delivery_from_cents":null,"delivery_postal_codes":[]
        }))
        .unwrap();
        assert_eq!(settings.banquet_minimum_quantity, 30);
    }
    #[test]
    fn delivery_respects_optional_threshold_and_exact_boundary() {
        let mut settings = Settings {
            delivery_fee_cents: 4900,
            free_delivery_from_cents: Some(35000),
            delivery_postal_codes: vec!["11000".into()],
            online_payment_enabled: false,
            ..Settings::default()
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
        assert_eq!(cart.get(&18), Some(&200));
        assert!(!cart.contains_key(&0));
        assert!(reconcile(
            &mut cart,
            &[product(18, 100, Some(3)), product(7, 100, Some(0))]
        ));
        assert_eq!(cart, Cart::from([(18, 3)]));
    }

    #[test]
    fn banquet_quantities_use_stock_and_the_database_limit() {
        let mut snack = product(18, 100, None);
        assert_eq!(snack.maximum(), 99);
        assert_eq!(snack.orderable_quantity(70000), Some(99));
        snack.stock = Some(70000);
        assert_eq!(snack.maximum(), 99);
        snack.banquet = true;
        snack.minimum_quantity = 120;
        assert_eq!(snack.orderable_quantity(1), Some(120));
        assert_eq!(snack.orderable_quantity(70001), Some(70000));
        snack.minimum_quantity = 70000;
        assert_eq!(snack.orderable_quantity(1), Some(70000));
        snack.stock = Some(69999);
        assert_eq!(snack.orderable_quantity(70000), None);
        snack.stock = None;
        assert_eq!(snack.maximum(), i32::MAX as u32);
        assert_eq!(snack.orderable_quantity(u32::MAX), Some(i32::MAX as u32));
        let restored: Product =
            serde_json::from_str(&serde_json::to_string(&snack).unwrap()).unwrap();
        assert_eq!(restored.minimum(), 70000);
    }

    #[test]
    fn restored_banquet_cart_keeps_large_quantities_until_product_reconciliation() {
        let mut products = vec![
            product(1, 100, None),
            product(2, 100, None),
            product(3, 100, None),
            product(4, 100, None),
        ];
        products[0].banquet = true;
        products[0].minimum_quantity = 120;
        products[1].banquet = true;
        products[1].minimum_quantity = 70000;
        products[3].banquet = true;
        let mut cart = parse_cart(r#"{"1":120,"2":70000,"3":70000,"4":4294967295}"#);
        assert_eq!(cart[&1], 120);
        assert_eq!(cart[&2], 70000);
        assert_eq!(cart[&4], i32::MAX as u32);
        assert!(reconcile(&mut cart, &products));
        assert_eq!(cart[&1], 120);
        assert_eq!(cart[&2], 70000);
        assert_eq!(cart[&3], 99);
        assert_eq!(cart[&4], i32::MAX as u32);
        assert!(!reconcile(&mut cart, &products));
        products[1].minimum_quantity = 120;
        products[1].stock = Some(60000);
        assert!(reconcile(&mut cart, &products));
        assert_eq!(cart[&2], 60000);
    }

    #[test]
    fn order_detail_and_request_preserve_quantities_above_u16() {
        let order: Order = serde_json::from_value(serde_json::json!({
            "id": "large-order", "number": "TEST-LARGE", "status": "new",
            "payment_method": "cash_on_delivery", "subtotal_cents": 7012000,
            "delivery_fee_cents": 0, "total_cents": 7012000,
            "items": [
                {"product_id": 1, "name": "Banketní pečivo", "quantity": 120,
                 "unit_price_cents": 100, "total_cents": 12000},
                {"product_id": 2, "name": "Banketní pečivo", "quantity": 70000,
                 "unit_price_cents": 100, "total_cents": 7000000}
            ]
        }))
        .unwrap();
        assert_eq!(order.items[0].quantity, 120);
        assert_eq!(order.items[1].quantity, 70000);
        let mut request = order_request();
        request.items[0].quantity = order.items[1].quantity;
        assert_eq!(
            serde_json::to_value(request).unwrap()["items"][0]["quantity"],
            70000
        );
    }

    #[test]
    fn large_cart_counts_and_prices_do_not_overflow() {
        let products: Vec<_> = (1..=50)
            .map(|id| {
                let mut item = product(id, 10000000, None);
                item.banquet = true;
                item
            })
            .collect();
        let cart: Cart = products
            .iter()
            .map(|product| (product.id, i32::MAX as u32))
            .collect();
        assert_eq!(quantity(&cart), 107374182350);
        assert_eq!(banquet_quantity(&cart, &products), 107374182350);
        assert_eq!(banquet_missing(&cart, &products, 4950), 0);
        assert_eq!(subtotal(&cart, &products), 1073741823500000000);
        let corrupt_products = [product(1, i64::MAX, None), product(2, i64::MAX, None)];
        assert_eq!(subtotal(&cart, &corrupt_products), i64::MAX);
        assert_eq!(
            product(1, i64::MIN, None).total_price_cents(u32::MAX),
            i64::MIN
        );
        assert_eq!(money(i64::MIN), "-92233720368547758,08 Kč");
    }
}

/// Normalize Czech search without requiring customers to type accents.
pub fn search_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'ä' => 'a',
            'č' => 'c',
            'ď' => 'd',
            'é' | 'ě' | 'ë' => 'e',
            'í' | 'ï' => 'i',
            'ň' => 'n',
            'ó' | 'ö' => 'o',
            'ř' => 'r',
            'š' => 's',
            'ť' => 't',
            'ú' | 'ů' | 'ü' => 'u',
            'ý' => 'y',
            'ž' => 'z',
            _ => c,
        })
        .collect()
}
pub fn allergen_label(value: &str) -> String {
    const LABELS: [&str; 14] = [
        "obiloviny obsahující lepek",
        "korýši",
        "vejce",
        "ryby",
        "arašídy",
        "sója",
        "mléko",
        "skořápkové plody",
        "celer",
        "hořčice",
        "sezam",
        "oxid siřičitý a siřičitany",
        "vlčí bob",
        "měkkýši",
    ];
    if value.trim().is_empty() {
        return "Informace zatím nebyly doplněny. Před objednáním si je prosím ověřte u obsluhy."
            .into();
    }
    value
        .split(',')
        .map(|part| {
            let t = part.trim();
            t.parse::<usize>()
                .ok()
                .filter(|i| (1..=14).contains(i))
                .map(|i| format!("{i} – {}", LABELS[i - 1]))
                .unwrap_or_else(|| t.into())
        })
        .collect::<Vec<_>>()
        .join(", ")
}
#[cfg(test)]
mod ux_tests {
    use super::*;
    #[test]
    fn czech_search_is_accent_and_space_insensitive() {
        assert_eq!(search_key("  ŠUNKA   & Sýr  "), "sunka & syr");
        assert!(search_key("Vajíčková pomazánka").contains(&search_key("vajickova")));
    }
    #[test]
    fn missing_allergens_are_never_presented_as_allergen_free() {
        assert!(allergen_label("").contains("nebyly doplněny"));
        assert_eq!(
            allergen_label("1, 3, 7"),
            "1 – obiloviny obsahující lepek, 3 – vejce, 7 – mléko"
        );
        assert_eq!(allergen_label("Obsahuje mléko"), "Obsahuje mléko");
    }
}
