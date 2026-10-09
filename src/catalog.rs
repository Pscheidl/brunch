//! Ukázkový katalog a výpočty košíku. Ceny jsou celé koruny.
#[derive(Clone, Copy)]
pub struct Product {
    pub id: usize,
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub tag: &'static str,
    pub weight: &'static str,
    pub price: u32,
    pub vegan: bool,
    pub ingredients: &'static str,
    pub allergens: &'static str,
}

pub const PRODUCTS: [Product; 6] = [
    Product {
        id: 0,
        name: "Avokádový kváskový sendvič",
        description: "Cottage, křupavá ředkvička a pořádná porce avokáda.",
        category: "Slané",
        tag: "Z naší pece",
        weight: "180 g",
        price: 119,
        vegan: false,
        ingredients: "Celozrnný kváskový chléb, avokádo, cottage, ředkvička, rukola, citron, semínka.",
        allergens: "Lepek (pšenice, žito), mléko, sezam.",
    },
    Product {
        id: 1,
        name: "Jogurt, granola & lesní ovoce",
        description: "Bílý jogurt, naše pečená granola a šťavnaté ovoce.",
        category: "Sladké",
        tag: "Ranní radost",
        weight: "220 g",
        price: 89,
        vegan: false,
        ingredients: "Bílý jogurt, ovesné vločky, med, mandle, maliny, borůvky.",
        allergens: "Mléko, lepek (oves), skořápkové plody (mandle).",
    },
    Product {
        id: 2,
        name: "Hummus bowl se zeleninou",
        description: "Jemný hummus, pečená cizrna a čerstvá zelenina.",
        category: "Bowls",
        tag: "Rostlinné",
        weight: "250 g",
        price: 129,
        vegan: true,
        ingredients: "Cizrna, sezamová pasta tahini, olivový olej, citron, okurka, mrkev, listový salát.",
        allergens: "Sezam.",
    },
    Product {
        id: 3,
        name: "Banánový chlebíček",
        description: "Vláčný, voňavý po banánech. S vlašskými ořechy.",
        category: "Sladké",
        tag: "Z naší pece",
        weight: "100 g",
        price: 65,
        vegan: true,
        ingredients: "Banány, pšeničná mouka, ovesný nápoj, vlašské ořechy, řepkový olej, třtinový cukr, skořice.",
        allergens: "Lepek (pšenice, oves), skořápkové plody (vlašské ořechy).",
    },
    Product {
        id: 4,
        name: "Celozrnný bagel s lučinou",
        description: "Čerstvý sýr, okurka a klíčky v měkkém bagelu.",
        category: "Slané",
        tag: "Z naší pece",
        weight: "160 g",
        price: 99,
        vegan: false,
        ingredients: "Celozrnný pšeničný bagel, čerstvý sýr, okurka, klíčky, sezam a lněné semínko.",
        allergens: "Lepek (pšenice), mléko, sezam.",
    },
    Product {
        id: 5,
        name: "Pomeranč, mrkev & zázvor",
        description: "Svěží ovocná šťáva s malým zázvorovým probuzením.",
        category: "Nápoje",
        tag: "Svěží doušek",
        weight: "250 ml",
        price: 69,
        vegan: true,
        ingredients: "Pomerančová šťáva, mrkvová šťáva, zázvor, citronová šťáva.",
        allergens: "Bez alergenů v ukázkové receptuře.",
    },
];

pub const FREE_DELIVERY_FROM: u32 = 350;
pub const DELIVERY_FEE: u32 = 49;
pub type Cart = [u16; 6];

pub fn subtotal(cart: &Cart) -> u32 {
    PRODUCTS
        .iter()
        .map(|p| p.price * u32::from(cart[p.id]))
        .sum()
}

pub fn delivery_fee(cart: &Cart) -> u32 {
    let amount = subtotal(cart);
    if amount == 0 || amount >= FREE_DELIVERY_FROM {
        0
    } else {
        DELIVERY_FEE
    }
}

pub fn quantity(cart: &Cart) -> u32 {
    cart.iter().map(|n| u32::from(*n)).sum()
}

pub fn parse_cart(value: &str) -> Cart {
    let parts: Vec<_> = value.split(',').collect();
    if parts.len() != PRODUCTS.len() {
        return [0; 6];
    }
    std::array::from_fn(|i| parts[i].parse::<u16>().unwrap_or(0).min(99))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prices_and_delivery_follow_cart_contents() {
        assert_eq!(delivery_fee(&[0; 6]), 0);
        let small = [1, 1, 0, 0, 0, 0];
        assert_eq!(subtotal(&small), 208);
        assert_eq!(subtotal(&small) + delivery_fee(&small), 257);
        let team = [2, 0, 1, 0, 0, 0];
        assert_eq!(subtotal(&team), 367);
        assert_eq!(delivery_fee(&team), 0);
        assert_eq!(quantity(&team), 3);
    }
    #[test]
    fn stored_cart_is_bounded_and_invalid_data_is_safe() {
        assert_eq!(parse_cart("2,1,0,0,0,3"), [2, 1, 0, 0, 0, 3]);
        assert_eq!(parse_cart("broken"), [0; 6]);
        assert_eq!(parse_cart("999,-1,no,2,0,1"), [99, 0, 0, 2, 0, 1]);
    }
}
