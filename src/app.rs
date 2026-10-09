use crate::{
    catalog::{self, Cart, FREE_DELIVERY_FROM, PRODUCTS, Product},
    icons::Icon,
};
use leptos::{html, prelude::*};

#[component]
pub fn App() -> impl IntoView {
    view! { <Home/> }
}

#[derive(Clone, Copy)]
struct Shop {
    cart: RwSignal<Cart>,
    delivery_dialog: NodeRef<html::Dialog>,
    cart_dialog: NodeRef<html::Dialog>,
    product_dialog: NodeRef<html::Dialog>,
    branches_dialog: NodeRef<html::Dialog>,
    selected_product: RwSignal<usize>,
    postal_code: RwSignal<String>,
    day: RwSignal<usize>,
    days: RwSignal<[String; 3]>,
    slot: RwSignal<String>,
    cart_step: RwSignal<u8>,
    cart_title: NodeRef<html::H2>,
    cart_content: NodeRef<html::Div>,
    draft_postal: RwSignal<String>,
    draft_day: RwSignal<usize>,
    draft_slot: RwSignal<String>,
    announcement: RwSignal<String>,
}

fn open_dialog(dialog: NodeRef<html::Dialog>) {
    if let Some(dialog) = dialog.get() {
        let _ = dialog.show_modal();
    }
}

fn close_dialog(dialog: NodeRef<html::Dialog>) {
    if let Some(dialog) = dialog.get() {
        dialog.close();
    }
}

impl Shop {
    fn open_delivery(self) {
        self.draft_postal.set(self.postal_code.get_untracked());
        self.draft_day.set(self.day.get_untracked());
        self.draft_slot.set(self.slot.get_untracked());
        open_dialog(self.delivery_dialog);
    }
    fn go_to_step(self, step: u8) {
        self.cart_step.set(step);
        if let Some(title) = self.cart_title.get() {
            let _ = title.focus();
        }
        if let Some(dialog) = self.cart_dialog.get() {
            dialog.set_scroll_top(0);
        }
        if let Some(content) = self.cart_content.get() {
            content.set_scroll_top(0);
        }
    }
    fn open_cart(self) {
        self.cart_step.set(0);
        open_dialog(self.cart_dialog);
    }
    fn add(self, id: usize) {
        self.cart
            .update(|cart| cart[id] = cart[id].saturating_add(1).min(99));
        self.announcement
            .set(format!("{} přidáno do košíku.", PRODUCTS[id].name));
    }
    fn show_product(self, id: usize) {
        self.selected_product.set(id);
        open_dialog(self.product_dialog);
    }
    fn delivery_label(self) -> String {
        format!("{}, {}", self.days.get()[self.day.get()], self.slot.get())
    }
}

#[component]
fn Home() -> impl IntoView {
    let shop = Shop {
        cart: RwSignal::new([0; 6]),
        delivery_dialog: NodeRef::new(),
        cart_dialog: NodeRef::new(),
        product_dialog: NodeRef::new(),
        branches_dialog: NodeRef::new(),
        selected_product: RwSignal::new(0),
        postal_code: RwSignal::new(String::new()),
        day: RwSignal::new(0),
        days: RwSignal::new([
            "Nejbližší pracovní den".into(),
            "Další pracovní den".into(),
            "Třetí pracovní den".into(),
        ]),
        slot: RwSignal::new("9:00–10:00".into()),
        cart_step: RwSignal::new(0),
        cart_title: NodeRef::new(),
        cart_content: NodeRef::new(),
        draft_postal: RwSignal::new(String::new()),
        draft_day: RwSignal::new(0),
        draft_slot: RwSignal::new("9:00–10:00".into()),
        announcement: RwSignal::new(String::new()),
    };
    let category = RwSignal::new("Vše");
    let vegan = RwSignal::new(false);
    let restored = RwSignal::new(false);

    Effect::new(move |_| {
        if !restored.get_untracked() {
            #[cfg(target_arch = "wasm32")]
            {
                if let Some(storage) =
                    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
                {
                    if let Ok(Some(saved)) = storage.get_item("brunch-cart-v1") {
                        shop.cart.set(catalog::parse_cart(&saved));
                    }
                }
                shop.days.set(next_delivery_days());
            }
            restored.set(true);
        }
    });
    Effect::new(move |_| {
        let ready = restored.get();
        let cart = shop.cart.get();
        if ready {
            #[cfg(target_arch = "wasm32")]
            if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
            {
                let value = cart
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let _ = storage.set_item("brunch-cart-v1", &value);
            }
            #[cfg(not(target_arch = "wasm32"))]
            let _ = cart;
        }
    });

    view! {
        <a class="skip-link" href="#nabidka">"Přejít na nabídku"</a>
        <div class="top-note">"Ochutnávka budoucího brunch. Prohlížíte si interaktivní návrh."</div>
        <header class="site-header">
            <div class="header-inner page-width">
                <a class="brand" href="./" aria-label="brunch, úvodní stránka"><span class="brand-mark"><Icon name="leaf"/></span>"brunch"<span class="brand-dot">"."</span></a>
                <nav class="desktop-nav" aria-label="Hlavní navigace">
                    <a href="#nabidka" class="active">"Naše nabídka"</a>
                    <a href="#jak-to-funguje">"Jak to funguje"</a>
                    <button class="nav-branches" on:click=move |_| open_dialog(shop.branches_dialog)>"Pobočky"</button>
                    <a href="#do-kancelare">"Pro vaši kancelář"</a>
                </nav>
                <div class="header-actions">
                    <button class="header-branches" on:click=move |_| open_dialog(shop.branches_dialog)><Icon name="pin"/><span>"Pobočky"</span></button>
                    <button class="header-cart" on:click=move |_| shop.open_cart() aria-label=move || format!("Otevřít košík, {} kusů", catalog::quantity(&shop.cart.get()))>
                        <Icon name="bag"/><span>"Košík"</span><span class="cart-count">{move || catalog::quantity(&shop.cart.get())}</span>
                    </button>
                </div>
            </div>
        </header>
        <main>
            <section class="hero page-width" aria-labelledby="hero-title">
                <div class="hero-copy">
                    <div class="eyebrow"><span class="status-dot"></span>"Z PEKÁRNY ROVNOU K VÁM"</div>
                    <h1 id="hero-title">"Dobrá pauza."<br/><span>"Lepší den."</span></h1>
                    <p>"Poctivé pečivo. Čerstvé suroviny. Svačiny, na které se budete těšit. Až k vám do kanceláře."</p>
                    <a class="button primary hero-cta" href="#nabidka">"Co si dnes dáte?"<Icon/></a>
                    <div class="hero-footnote"><Icon name="check"/>"Pár kliknutí. Bez registrace."</div>
                </div>
                <div class="hero-visual">
                    <img src="images/brunch-spread.webp" alt="Kváskový sendvič, jogurt s granolou a hummus se zeleninou na prostřeném stole" width="1536" height="1024" fetchpriority="high"/>
                    <div class="photo-caption"><span class="caption-icon"><Icon name="sun"/></span><div><strong>"Malá pauza. Velká radost."</strong><span>"Dobré věci si zaslouží svůj čas."</span></div></div>
                    <span class="photo-index">"01 / DOBRÉ RÁNO"</span>
                </div>
            </section>

            <div class="page-width">
                <section class="delivery-strip" aria-label="Nastavení doručení">
                    <button class="delivery-selector" on:click=move |_| shop.open_delivery()>
                        <Icon name="pin"/><span><small>"VAŠE KANCELÁŘ"</small><strong>{move || if shop.postal_code.get().is_empty() { "Kam to přivezeme?".into() } else { format!("PSČ {}", shop.postal_code.get()) }}</strong></span><Icon name="chevron"/>
                    </button>
                    <button class="delivery-selector" on:click=move |_| shop.open_delivery()>
                        <Icon name="clock"/><span><small>"UKÁZKOVÝ TERMÍN"</small><strong>{move || shop.delivery_label()}</strong></span><Icon name="chevron"/>
                    </button>
                    <div class="delivery-price"><Icon name="truck"/><span>"Doručení "<strong>"49 Kč"</strong><small>"Od 350 Kč ho máte od nás."</small></span></div>
                </section>
                <div class="benefits"><span><Icon name="leaf"/>"Suroviny, které znáte"</span><span><Icon name="sun"/>"Připraveno na vaše ráno"</span><span><Icon name="bag"/>"Pro vás i celý tým"</span></div>
            </div>

            <section class="catalog page-width" id="nabidka" aria-labelledby="catalog-title">
                <div class="section-heading"><div><p class="eyebrow">"VAŠE DALŠÍ OBLÍBENÁ SVAČINA"</p><h2 id="catalog-title">"Na co máte chuť?"</h2></div><p class="catalog-note">"Něco malého. Něco dobrého. Něco pro vás."</p></div>
                <div class="filter-bar">
                    <div class="categories" role="group" aria-label="Kategorie svačin">
                        {["Vše", "Slané", "Sladké", "Bowls", "Nápoje"].into_iter().map(move |name| view! {
                            <button class:active=move || category.get() == name aria-pressed=move || category.get() == name on:click=move |_| category.set(name)>{name}</button>
                        }).collect_view()}
                    </div>
                    <label class="vegan-filter"><input type="checkbox" prop:checked=move || vegan.get() on:change=move |ev| vegan.set(event_target_checked(&ev))/><Icon name="leaf"/>"Jen rostlinné"</label>
                </div>
                <div class="result-meta" role="status">{move || {
                    let count = PRODUCTS.iter().filter(|p| (category.get() == "Vše" || category.get() == p.category) && (!vegan.get() || p.vegan)).count();
                    format!("{} {}", count, if count == 1 { "svačina v nabídce" } else if (2..=4).contains(&count) { "svačiny v nabídce" } else { "svačin v nabídce" })
                }}</div>
                <div class="product-grid">
                    {PRODUCTS.into_iter().map(move |product| view! {
                        <div hidden=move || (category.get() != "Vše" && category.get() != product.category) || (vegan.get() && !product.vegan)>
                            <ProductCard product shop/>
                        </div>
                    }).collect_view()}
                </div>
                <Show when=move || !PRODUCTS.iter().any(|p| (category.get() == "Vše" || category.get() == p.category) && (!vegan.get() || p.vegan))>
                    <div class="empty-results"><Icon name="leaf"/><h3>"Tuhle kombinaci zatím nepečeme."</h3><p>"Zkuste jinou kategorii nebo si prohlédněte celou nabídku."</p><button class="button secondary" on:click=move |_| {category.set("Vše"); vegan.set(false);}>"Zobrazit všechny svačiny"</button></div>
                </Show>
                <p class="catalog-disclaimer">"Ukázková nabídka a ceny včetně DPH. Fotografie jsou ilustrační."</p>
            </section>

            <section class="how-section page-width" id="jak-to-funguje" aria-labelledby="how-title">
                <div class="section-heading"><div><p class="eyebrow">"PAUZA BEZ STAROSTÍ"</p><h2 id="how-title">"Vy máte svůj den. My svačinu."</h2></div></div>
                <div class="steps-grid">
                    <article><span class="step-number">"01"</span><h3>"Vyberte si podle chuti"</h3><p>"Slané, sladké nebo něco do misky. Složení i alergeny najdete u každé svačiny."</p></article>
                    <article><span class="step-number">"02"</span><h3>"Řekněte nám kam a kdy"</h3><p>"Zvolte si ranní čas, který vám sedí. Doručení i celkovou cenu uvidíte předem."</p></article>
                    <article><span class="step-number">"03"</span><h3>"Udělejte si dobrou pauzu"</h3><p>"Přivezeme ji až do kanceláře. Stačí zavřít notebook a na chvíli si vydechnout."</p></article>
                </div>
            </section>
            <section class="office-section page-width" id="do-kancelare" aria-labelledby="office-title"><div class="office-banner"><div><span class="eyebrow">"DOBRÉ VĚCI SE LÉPE SDÍLÍ"</span><h2 id="office-title">"Přiberte ke svačině kolegy."</h2><p>"Každému něco podle chuti. Jedna společná objednávka, jedna cesta k vám. Od 350 Kč s doručením zdarma."</p></div><a class="button secondary" href="#nabidka">"Vybrat pro celý tým"<Icon/></a></div></section>
        </main>
        <footer class="site-footer page-width"><a class="brand footer-brand" href="./">"brunch."</a><p>"Dobrá pauza začíná tady."</p><button class="text-button footer-branches" on:click=move |_| open_dialog(shop.branches_dialog)>"Pobočky"<Icon name="pin"/></button><span>"Návrh · 2026"</span></footer>
        <div class="mobile-cart" class:has-items={move || catalog::quantity(&shop.cart.get()) > 0}><button class="button primary" on:click=move |_| shop.open_cart()><span><Icon name="bag"/>"Zobrazit košík"<span class="cart-count">{move || catalog::quantity(&shop.cart.get())}</span></span><strong>{move || format!("{} Kč", catalog::subtotal(&shop.cart.get()) + catalog::delivery_fee(&shop.cart.get()))}<Icon/></strong></button></div>
        <div class="sr-only" aria-live="polite" aria-atomic="true">{move || shop.announcement.get()}</div>
        <DeliveryDialog shop/>
        <ProductDialog shop/>
        <CartDialog shop/>
        <BranchesDialog shop/>
    }
}

#[component]
fn ProductCard(product: Product, shop: Shop) -> impl IntoView {
    view! {
        <article class="product-card" class:in-cart={move || shop.cart.get()[product.id] > 0}>
            <button class=format!("product-photo snack-image snack-{}", product.id) aria-label=format!("Podrobnosti: {}", product.name) on:click=move |_| shop.show_product(product.id)>
                <span class="product-tag">{product.tag}</span><span class="photo-detail">"Složení a alergeny"<Icon name="plus"/></span>
            </button>
            <div class="product-info"><div class="product-meta"><span>{product.category}</span><span>{product.weight}</span></div>
                <h3><button on:click=move |_| shop.show_product(product.id)>{product.name}</button></h3><p>{product.description}</p>
                <div class="product-bottom"><span class="product-price">{product.price}<small>" Kč"</small></span><QuantityControl id=product.id shop/></div>
            </div>
        </article>
    }
}

#[component]
fn QuantityControl(id: usize, shop: Shop) -> impl IntoView {
    view! {
        <Show when={move || shop.cart.get()[id] > 0} fallback=move || view! {
            <button class="add-button" aria-label=format!("Přidat {} do košíku", PRODUCTS[id].name) on:click=move |_| shop.add(id)>"Přidat"<Icon name="plus"/></button>
        }>
            <div class="quantity-control" role="group" aria-label=format!("Počet: {}", PRODUCTS[id].name)>
                <button aria-label=format!("Odebrat jeden kus: {}", PRODUCTS[id].name) on:click=move |_| shop.cart.update(|c| c[id] = c[id].saturating_sub(1))><Icon name="minus"/></button>
                <span aria-live="polite">{move || shop.cart.get()[id]}</span>
                <button aria-label=format!("Přidat jeden kus: {}", PRODUCTS[id].name) disabled={move || shop.cart.get()[id] >= 99} on:click=move |_| shop.add(id)><Icon name="plus"/></button>
            </div>
        </Show>
    }
}

#[component]
fn DeliveryDialog(shop: Shop) -> impl IntoView {
    let draft_postal = shop.draft_postal;
    let draft_day = shop.draft_day;
    let draft_slot = shop.draft_slot;
    view! {
        <dialog node_ref=shop.delivery_dialog class="delivery-dialog" aria-labelledby="delivery-title">
            <div class="dialog-heading"><div><p class="eyebrow">"DOBRÁ PAUZA V PRAVÝ ČAS"</p><h2 id="delivery-title">"Kam a kdy to bude?"</h2></div><button class="icon-button" aria-label="Zavřít nastavení doručení" on:click=move |_| close_dialog(shop.delivery_dialog)><Icon name="close"/></button></div>
            <p class="dialog-intro">"Vyberte si doručení do kanceláře. Vše můžete ještě změnit v košíku."</p>
            <form on:submit=move |ev| {
                ev.prevent_default();
                shop.postal_code.set(draft_postal.get().trim().to_owned());
                shop.day.set(draft_day.get());
                shop.slot.set(draft_slot.get());
                close_dialog(shop.delivery_dialog);
            }>
                <label class="form-field">"PSČ kanceláře"<input name="delivery-postal" autocomplete="postal-code" inputmode="numeric" placeholder="Např. 110 00" pattern="[0-9]{3} ?[0-9]{2}" title="Zadejte pět číslic, například 110 00" required prop:value=move || draft_postal.get() on:input=move |ev| draft_postal.set(event_target_value(&ev))/></label>
                <fieldset><legend>"Den doručení"</legend><div class="day-options">{(0..3).map(move |index| view! {<label class:selected=move || draft_day.get() == index><input type="radio" name="delivery-day" prop:checked=move || draft_day.get() == index on:change=move |_| draft_day.set(index)/><span>{move || shop.days.get()[index].clone()}</span></label>}).collect_view()}</div></fieldset>
                <label class="form-field">"Čas doručení"<select name="delivery-time" prop:value=move || draft_slot.get() on:change=move |ev| draft_slot.set(event_target_value(&ev))><option>"8:00–9:00"</option><option>"9:00–10:00"</option><option>"10:00–11:00"</option></select></label>
                <div class="info-note"><Icon name="truck"/><p>"Doručení 49 Kč. Od 350 Kč zdarma."</p></div>
                <p class="demo-note">"Ukázkové nastavení. PSČ zatím neověřuje oblast rozvozu a termín se nerezervuje."</p>
                <button class="button primary full-width" type="submit">"Uložit a vybírat"<Icon/></button>
            </form>
        </dialog>
    }
}

#[component]
fn ProductDialog(shop: Shop) -> impl IntoView {
    view! {
        <dialog node_ref=shop.product_dialog class="product-dialog" aria-labelledby="product-title">
            <button class="icon-button floating-close" aria-label="Zavřít detail svačiny" on:click=move |_| close_dialog(shop.product_dialog)><Icon name="close"/></button>
            {move || {
                let product = PRODUCTS[shop.selected_product.get()];
                view! {
                    <div class=format!("detail-photo snack-image snack-{}", product.id) role="img" aria-label=product.name></div>
                    <div class="detail-content"><div class="product-meta"><span>{product.tag}</span><span>{product.weight}</span></div><h2 id="product-title">{product.name}</h2><p>{product.description}</p>
                        <h3>"Co je uvnitř"</h3><p>{product.ingredients}</p><h3>"Alergeny"</h3><p>{product.allergens}</p>
                        <p class="demo-note">"Ukázková receptura a ilustrační fotografie. Finální složení a alergeny potvrdí pekárna před spuštěním."</p>
                        <div class="product-bottom"><strong class="product-price">{product.price}" Kč"</strong><QuantityControl id=product.id shop/></div>
                    </div>
                }
            }}
        </dialog>
    }
}

#[component]
fn CartTotals(shop: Shop) -> impl IntoView {
    view! {
        <div class="cart-totals"><div><span>"Svačiny"</span><span>{move || format!("{} Kč", catalog::subtotal(&shop.cart.get()))}</span></div><div><span>"Doručení"</span><span>{move || if catalog::delivery_fee(&shop.cart.get()) == 0 { "Zdarma" } else { "49 Kč" }}</span></div><div class="grand-total"><strong>"Celkem"<small>"včetně DPH"</small></strong><strong>{move || format!("{} Kč", catalog::subtotal(&shop.cart.get()) + catalog::delivery_fee(&shop.cart.get()))}</strong></div></div>
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PaymentMethod {
    Card,
    OnDelivery,
}

#[derive(Clone, Copy)]
struct CheckoutDetails {
    name: RwSignal<String>,
    email: RwSignal<String>,
    phone: RwSignal<String>,
    organization: RwSignal<String>,
    street: RwSignal<String>,
    city: RwSignal<String>,
}

#[component]
fn CheckoutStepper(shop: Shop) -> impl IntoView {
    view! {
        <ol class="checkout-stepper" aria-label="Kroky objednávky" hidden=move || shop.cart_step.get() == 3>
            {[(0, "Košík"), (1, "Doručení"), (2, "Platba")].into_iter().map(move |(step, label)| view! {
                <li class:current=move || shop.cart_step.get() == step class:complete={move || shop.cart_step.get() > step}>
                    <button type="button" disabled={move || step >= shop.cart_step.get()} aria-current=move || (shop.cart_step.get() == step).then_some("step") on:click=move |_| shop.go_to_step(step)>
                        <span class="step-dot"><Show when={move || shop.cart_step.get() > step} fallback=move || view! { {step + 1} }><Icon name="check"/></Show></span>
                        <span>{label}</span>
                    </button>
                </li>
            }).collect_view()}
        </ol>
    }
}

#[component]
fn CheckoutAddress(shop: Shop, details: CheckoutDetails) -> impl IntoView {
    view! {
        <div class="checkout-address">
            <Icon name="pin"/>
            <div>
                <strong>{move || details.name.get()}</strong>
                <Show when=move || !details.organization.get().trim().is_empty()><span>{move || details.organization.get()}</span></Show>
                <span>{move || details.street.get()}</span>
                <span>{move || format!("{} {}", shop.postal_code.get(), details.city.get())}</span>
            </div>
        </div>
        <div class="checkout-address"><Icon name="clock"/><div><strong>{move || shop.delivery_label()}</strong><span>"Ukázkový termín doručení"</span></div></div>
    }
}

#[component]
fn CheckoutItems(shop: Shop) -> impl IntoView {
    view! {
        <div class="checkout-review-items">
            {PRODUCTS.into_iter().map(move |product| view! {
                <Show when={move || shop.cart.get()[product.id] > 0}>
                    <div><span><b>{move || shop.cart.get()[product.id]}"×"</b>{product.name}</span><strong>{move || format!("{} Kč", product.price * u32::from(shop.cart.get()[product.id]))}</strong></div>
                </Show>
            }).collect_view()}
        </div>
    }
}

#[component]
fn CartDialog(shop: Shop) -> impl IntoView {
    let details = CheckoutDetails {
        name: RwSignal::new(String::new()),
        email: RwSignal::new(String::new()),
        phone: RwSignal::new(String::new()),
        organization: RwSignal::new(String::new()),
        street: RwSignal::new(String::new()),
        city: RwSignal::new(String::new()),
    };
    let payment_method = RwSignal::new(PaymentMethod::Card);
    view! {
        <dialog node_ref=shop.cart_dialog class="cart-dialog checkout-dialog" aria-labelledby="cart-title">
            <div class="dialog-heading">
                <div><p class="eyebrow">"VAŠE DOBRÁ PAUZA"</p><h2 id="cart-title" node_ref=shop.cart_title tabindex="-1">{move || match shop.cart_step.get() { 1 => "Kam to přivezeme?", 2 => "Už jen poslední krok.", 3 => "Ukázka je hotová.", _ => "Váš košík" }}</h2></div>
                <button class="icon-button" aria-label="Zavřít košík" on:click=move |_| close_dialog(shop.cart_dialog)><Icon name="close"/></button>
            </div>
            <Show when={move || catalog::quantity(&shop.cart.get()) > 0} fallback=move || view! {
                <div class="empty-cart"><span class="empty-cart-icon"><Icon name="bag"/></span><h3>"Vaše pauza je zatím prázdná."</h3><p>"Přidejte si něco dobrého. Třeba čerstvý sendvič nebo jogurt s granolou."</p><button class="button primary" on:click=move |_| close_dialog(shop.cart_dialog)>"Jdu si vybrat"<Icon/></button></div>
            }>
                <CheckoutStepper shop/>
                <div class="checkout-body" node_ref=shop.cart_content>
                    <section data-checkout-step="cart" hidden=move || shop.cart_step.get() != 0 aria-label="Obsah košíku">
                        <div class="free-delivery"><Icon name="truck"/><p>{move || {let amount = catalog::subtotal(&shop.cart.get()); if amount >= FREE_DELIVERY_FROM { "Doručení máte od nás. Dobrou chuť!".into() } else { format!("Ještě {} Kč a doručení máte zdarma.", FREE_DELIVERY_FROM - amount) }}}</p><progress max=FREE_DELIVERY_FROM value=move || catalog::subtotal(&shop.cart.get()).min(FREE_DELIVERY_FROM) aria-label="Do dopravy zdarma"/></div>
                        <div class="cart-items">{PRODUCTS.into_iter().map(move |product| view! {
                            <Show when={move || shop.cart.get()[product.id] > 0}>
                                <article class="cart-item"><div class=format!("cart-item-photo snack-image snack-{}", product.id) role="img" aria-label=product.name></div><div class="cart-item-info"><h3>{product.name}</h3><p>{product.weight}" · "{product.price}" Kč / ks"</p><div class="cart-item-actions"><QuantityControl id=product.id shop/><strong>{move || format!("{} Kč", u32::from(shop.cart.get()[product.id]) * product.price)}</strong></div></div></article>
                            </Show>
                        }).collect_view()}</div>
                        <div class="cart-delivery"><Icon name="clock"/><div><strong>{move || shop.delivery_label()}</strong><span>{move || if shop.postal_code.get().is_empty() { "Adresu doplníte v dalším kroku".into() } else { format!("Doručení na PSČ {}", shop.postal_code.get()) }}</span></div><button class="text-button" on:click=move |_| shop.go_to_step(1)>"Upravit"</button></div>
                        <CartTotals shop/>
                        <button class="button primary full-width" on:click=move |_| shop.go_to_step(1)>"Pokračovat k doručení"<Icon/></button>
                        <p class="checkout-reassurance"><Icon name="check"/>"Bez registrace. Za pár chvil hotovo."</p>
                    </section>

                    <section data-checkout-step="delivery" hidden=move || shop.cart_step.get() != 1 aria-label="Doručovací údaje">
                        <div class="checkout-guest"><span class="checkout-guest-icon"><Icon name="person"/></span><div><strong>"Objednáváte bez účtu"</strong><span>"Stačí nám vědět, komu a kam přivézt svačinu."</span></div></div>
                        <div class="checkout-demo-help"><p>"Zkoušíte si návrh. Použijte klidně smyšlené údaje."</p><button class="text-button" type="button" on:click=move |_| {
                            details.name.set("Jana Ukázková".into());
                            details.email.set("demo@example.com".into());
                            details.phone.set("+420 000 000 000".into());
                            details.organization.set("Ukázková kancelář, 2. patro".into());
                            details.street.set("Ukázková 12".into());
                            details.city.set("Ukázkové město".into());
                            shop.postal_code.set("110 00".into());
                        }>"Vyplnit ukázkové údaje"<Icon name="sparkle"/></button></div>
                        <form class="checkout-form" on:submit=move |ev| { ev.prevent_default(); shop.go_to_step(2); }>
                            <fieldset class="checkout-fieldset"><legend>"Kontakt na vás"</legend>
                                <label class="form-field">"Jméno a příjmení"<input name="name" autocomplete="name" pattern=r".*\S.*" title="Vyplňte jméno a příjmení" required prop:value=move || details.name.get() on:input=move |ev| details.name.set(event_target_value(&ev))/></label>
                                <div class="form-row"><label class="form-field">"E-mail"<input type="email" name="email" autocomplete="email" required prop:value=move || details.email.get() on:input=move |ev| details.email.set(event_target_value(&ev))/></label><label class="form-field">"Telefon pro kurýra"<input type="tel" name="tel" autocomplete="tel" minlength="9" pattern=r"\+?(?:[0-9] *){9,15}" title="Zadejte 9 až 15 číslic, například +420 777 123 456" required prop:value=move || details.phone.get() on:input=move |ev| details.phone.set(event_target_value(&ev))/></label></div>
                            </fieldset>
                            <fieldset class="checkout-fieldset"><legend>"Adresa kanceláře"</legend>
                                <label class="form-field">"Ulice a číslo domu"<input name="street-address" autocomplete="street-address" pattern=r".*\S.*" title="Vyplňte ulici a číslo domu" required prop:value=move || details.street.get() on:input=move |ev| details.street.set(event_target_value(&ev))/></label>
                                <div class="form-row address-row"><label class="form-field">"Město"<input name="city" autocomplete="address-level2" pattern=r".*\S.*" title="Vyplňte město" required prop:value=move || details.city.get() on:input=move |ev| details.city.set(event_target_value(&ev))/></label><label class="form-field">"PSČ"<input name="postal-code" autocomplete="postal-code" inputmode="numeric" pattern="[0-9]{3} ?[0-9]{2}" title="Zadejte pět číslic, například 110 00" required prop:value=move || shop.postal_code.get() on:input=move |ev| shop.postal_code.set(event_target_value(&ev))/></label></div>
                                <label class="form-field">"Firma, patro nebo zvonek (nepovinné)"<input name="organization" autocomplete="organization" placeholder="Ať vás snadno najdeme" prop:value=move || details.organization.get() on:input=move |ev| details.organization.set(event_target_value(&ev))/></label>
                            </fieldset>
                            <fieldset class="checkout-fieldset"><legend>"Kdy se vám to hodí?"</legend>
                                <div class="day-options">{(0..3).map(move |index| view! {<label class:selected=move || shop.day.get() == index><input type="radio" name="checkout-day" prop:checked=move || shop.day.get() == index on:change=move |_| shop.day.set(index)/><span>{move || shop.days.get()[index].clone()}</span></label>}).collect_view()}</div>
                                <label class="form-field checkout-time">"Čas doručení"<select name="checkout-time" prop:value=move || shop.slot.get() on:change=move |ev| shop.slot.set(event_target_value(&ev))><option>"8:00–9:00"</option><option>"9:00–10:00"</option><option>"10:00–11:00"</option></select></label>
                                <p class="field-hint">"Ukázkové termíny. Oblast rozvozu ani volnou kapacitu zatím neověřujeme."</p>
                            </fieldset>
                            <div class="checkout-next-total"><span>"Celkem včetně doručení"</span><strong>{move || format!("{} Kč", catalog::subtotal(&shop.cart.get()) + catalog::delivery_fee(&shop.cart.get()))}</strong></div>
                            <button class="button primary full-width" type="submit">"Pokračovat k platbě"<Icon/></button>
                            <button class="text-button checkout-back" type="button" on:click=move |_| shop.go_to_step(0)><Icon name="back"/>"Zpět do košíku"</button>
                        </form>
                    </section>

                    <section data-checkout-step="payment" hidden=move || shop.cart_step.get() != 2 aria-label="Platba a kontrola objednávky">
                        <div class="checkout-review">
                            <div class="checkout-section-heading"><h3>"Vaše dobrá pauza"</h3><button class="text-button" type="button" on:click=move |_| shop.go_to_step(1)>"Upravit doručení"</button></div>
                            <CheckoutAddress shop details/>
                            <CheckoutItems shop/>
                        </div>
                        <fieldset class="payment-options"><legend>"Jak chcete zaplatit?"</legend>
                            <label class="payment-option" class:selected=move || payment_method.get() == PaymentMethod::Card>
                                <input type="radio" name="payment-method" value="card" prop:checked=move || payment_method.get() == PaymentMethod::Card on:change=move |_| payment_method.set(PaymentMethod::Card)/>
                                <Icon name="card"/><span><strong>"Kartou online"</strong><small>"Vyzkoušejte ukázkovou platbu"</small></span><span class="payment-fee">"Zdarma"</span>
                            </label>
                            <label class="payment-option" class:selected=move || payment_method.get() == PaymentMethod::OnDelivery>
                                <input type="radio" name="payment-method" value="on-delivery" prop:checked=move || payment_method.get() == PaymentMethod::OnDelivery on:change=move |_| payment_method.set(PaymentMethod::OnDelivery)/>
                                <Icon name="wallet"/><span><strong>"Při převzetí"</strong><small>"Ukázka úhrady kurýrovi"</small></span><span class="payment-fee">"Zdarma"</span>
                            </label>
                        </fieldset>
                        <div class="demo-payment-card" hidden=move || payment_method.get() != PaymentMethod::Card>
                            <div class="demo-card-top"><span>"TESTOVACÍ KARTA"</span><Icon name="card"/></div><strong class="demo-card-number">"••••  ••••  ••••  4242"</strong><div class="demo-card-bottom"><span>"Připravená k vyzkoušení"</span><b>"DEMO"</b></div>
                        </div>
                        <div class="payment-demo-note"><Icon name="info"/><p><strong>"Toto je pouze ukázka."</strong>" Nezadáváte údaje skutečné karty. Objednávka se neodešle a nic nezaplatíte."</p></div>
                        <CartTotals shop/>
                        <button class="button primary full-width payment-submit" type="button" on:click=move |_| shop.go_to_step(3)><Icon name="check"/>{move || format!("{} · {} Kč", if payment_method.get() == PaymentMethod::Card { "Vyzkoušet platbu" } else { "Dokončit ukázku" }, catalog::subtotal(&shop.cart.get()) + catalog::delivery_fee(&shop.cart.get()))}</button>
                        <button class="text-button checkout-back" type="button" on:click=move |_| shop.go_to_step(1)><Icon name="back"/>"Zpět k doručení"</button>
                    </section>

                    <section data-checkout-step="confirmation" hidden=move || shop.cart_step.get() != 3 class="checkout-confirmation" aria-label="Potvrzení ukázkové objednávky">
                        <div class="confirmation-intro"><span class="confirmation-mark"><Icon name="check"/></span><p class="eyebrow">"TAKHLE JEDNODUCHÉ TO BUDE"</p><h3>"Dobrá pauza je na dohled."</h3><p>"Prošli jste celou ukázkovou objednávkou."<br/><strong>"Nic se neobjednalo ani nezaplatilo."</strong></p></div>
                        <div class="confirmation-details"><h4>"Vaše ukázková objednávka"</h4><CheckoutAddress shop details/><div class="checkout-address"><Icon name="card"/><div><strong>{move || if payment_method.get() == PaymentMethod::Card { "Kartou online · ukázka" } else { "Při převzetí · ukázka" }}</strong><span>{move || details.email.get()}</span></div></div><CheckoutItems shop/><CartTotals shop/></div>
                        <p class="confirmation-note">"Žádný e-mail jsme neposlali. Vyplněné údaje zůstávají pouze v této otevřené stránce."</p>
                        <button class="button primary full-width" on:click=move |_| {shop.go_to_step(0); close_dialog(shop.cart_dialog);}>"Zpátky na svačiny"<Icon/></button>
                        <button class="text-button checkout-back" type="button" on:click=move |_| shop.go_to_step(0)>"Prohlédnout košík znovu"</button>
                    </section>
                </div>
            </Show>
        </dialog>
    }
}

#[component]
fn BranchesDialog(shop: Shop) -> impl IntoView {
    view! {
        <dialog node_ref=shop.branches_dialog class="branches-dialog" aria-labelledby="branches-title">
            <div class="dialog-heading"><div><p class="eyebrow">"POTKEJME SE U NĚČEHO DOBRÉHO"</p><h2 id="branches-title">"Naše pobočky"</h2></div><button class="icon-button" aria-label="Zavřít pobočky" on:click=move |_| close_dialog(shop.branches_dialog)><Icon name="close"/></button></div>
            <div class="branches-illustration"><Icon name="store"/><span>"Brzy i osobně"</span></div>
            <p class="branches-description">"Tady najdete místa, kde si můžete dát svou dobrou pauzu. Konkrétní pobočky, adresy a otevírací dobu právě připravujeme."</p>
            <a class="button primary full-width" href="#nabidka" on:click=move |_| close_dialog(shop.branches_dialog)>"Zatím si vybrat svačinu"<Icon/></a>
        </dialog>
    }
}

#[cfg(target_arch = "wasm32")]
fn next_delivery_days() -> [String; 3] {
    let date = js_sys::Date::new_0();
    let weekdays = ["Ne", "Po", "Út", "St", "Čt", "Pá", "So"];
    std::array::from_fn(|_| {
        loop {
            date.set_date(date.get_date() + 1);
            if date.get_day() != 0 && date.get_day() != 6 {
                break;
            }
        }
        format!(
            "{} {}. {}.",
            weekdays[date.get_day() as usize],
            date.get_date(),
            date.get_month() + 1
        )
    })
}
