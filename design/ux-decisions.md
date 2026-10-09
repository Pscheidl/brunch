# Rozhodnutí pro první návrh frontendu

Stav k 9. 10. 2026. První etapa ověřuje výběr svačin a podobu objednávkového toku. Jde o interaktivní návrh s ukázkovými daty, nikoli o spuštěný rozvoz.

## Vzhled a hlavní cesta

Přebíráme základ projektu `obecni-web`: Leptos a lokální písmo Geist, jemné linky a světlé plochy. Aktuální paleta kombinuje elegantní bílý základ (#ffffff), teplou olivovou (#5d7144), světlou šalvěj (#edf2e5) a meruňkový doplněk (#f4cfae). Hlavní text používá #293025 a vedlejší #626d5b. Olivová vede hlavní akce a vybrané možnosti. Jemný přechod ze šalvěje do světlé meruňkové propojuje doručovací panel a banner pro kanceláře, podobné barvy rámují fotografii. Nadpis přechází z olivové do teplého odstínu #817344. Meruňková doplňuje logo, sluneční ikonu a štítky produktů. Fotografie jídla, větší produktové karty a přímé ovládání množství přizpůsobují tento základ pekárně.

Hlavní cesta je nabídka → košík → doručovací údaje → ukázková platba → potvrzení prototypu. Objednávka má tři označené kroky, mezi kterými se lze vracet bez ztráty kontaktních údajů. Platební krok ukazuje konečnou cenu a umožňuje porovnat kartu online s platbou při převzetí. Údaje skutečné platební karty se nezadávají. Kategorie a filtr rostlinných svačin zužují nabídku. Detail zpřístupňuje složení a alergeny. Košík přepočítává cenu průběžně a uchovává pouze počty produktů v místním úložišti prohlížeče. Kontaktní údaje zůstávají v aktuální stránce a nikam se neodesílají.

## Výzkumný podklad palety

- **Zelená jako vodítko pro vnímání zdravějšího jídla.** Schuldt (2013), [Does Green Mean Healthy?](https://doi.org/10.1080/10410236.2012.725270), zjistil u identické sladké tyčinky rozdíl ve vnímané zdravosti podle zeleného či červeného kalorického štítku. Srovnání se bílou záviselo také na důležitosti zdravého stravování pro účastníka. Jde o vnímání obalů a štítků, nikoli o nutriční vlastnosti potraviny nebo měření webu. Zelená rodina barev je tímto výzkumem inspirovaná, konkrétní olivový odstín je naše návrhová volba.
- **Sytost nemá univerzální účinek.** Mead a Richerson (2018), [Package color saturation and food healthfulness perceptions](https://www.sciencedirect.com/science/article/pii/S0148296317302783), ve čtyřech experimentech zjistili nižší vnímanou zdravost u sytějších obalů. Kunz a kol. (2020), [Fifty shades of food](https://doi.org/10.1002/mar.21317), naopak u obrázků obalů nápojů naměřili vyšší vnímanou čerstvost a očekávanou chutnost při zvýšené sytosti, s menším kladným účinkem na vnímanou zdravost. Z těchto výsledků nelze odvodit jednu nejúčinnější paletu pro brunch.
- **Převod do návrhu.** Bílou a tlumené zelené plochy doplňuje teplá meruňka, která navazuje na pečivo a suroviny na fotografiích. Tento doplněk i použití přechodů jsou designová interpretace. Studie neověřovaly tyto HEX barvy, jejich kombinaci ani vliv na objednávky na našem webu. Fotografie zůstávají barevné a složení i alergeny dostupné v detailu.

Bílý text na hlavní olivové má kontrast 5,37 : 1, olivový text na šalvějové 4,71 : 1 a vedlejší text na šalvějové 4,78 : 1. Hlavní tlačítka mají plnou barvu, přechody patří převážně dekorativním plochám. Nadpis má v režimu vynucených systémových barev čitelný jednobarevný fallback.

Paleta ověřena 9. 10. 2026 v prohlížeči na běžné šířce 1280 px a mobilní šířce 390 px bez vodorovného přetékání. Zkontrolován úvod, nabídka, přidání a odebrání produktu i kontrast mobilního odznaku košíku (8,15 : 1). Minimum kontrastu přechodu nadpisu vůči bílé je 4,70 : 1. Snímky jsou `design/previews/olive-apricot-desktop.png` a `design/previews/olive-apricot-mobile.png`. Režim vynucených barev byl ošetřen ve stylech, není zahrnut do této vizuální kontroly.

## Podklady pro rozhodnutí

- **Přidání přímo z nabídky.** Po přidání produktu se tlačítko změní na ovládání počtu kusů. Zákazník vidí obsah košíku přímo při výběru a může snadno přidat svačinu kolegovi. Tento vzor popisuje výzkum nákupu potravin [Baymard: Grocery Add to Cart Buttons](https://baymard.com/research-articles/grocery-add-to-cart-buttons), publikovaný 12. 9. 2023.
- **Průchod bez účtu a konkrétní termín.** Registrace není součástí současného návrhu. Datum a čas doručení zobrazujeme v nabídce i košíku. Opíráme se o veřejná doporučení pro nákup bez registrace, srozumitelné datum doručení a ovládání množství v článku [Baymard: Checkout UX 2025](https://baymard.com/research-articles/current-state-of-checkout-ux), aktualizovaném 25. 11. 2025.
- **Doručení viditelné před naplněním košíku.** Veřejně dostupný popis problému [Baymard: Fulfillment Time Slot Availability #1955](https://baymard.com/guidelines/1955-fulfillment-time-slot-availability) upozorňuje na zklamání zákazníků, kteří nevhodné termíny zjistí až po výběru zboží. Naším návrhovým závěrem je umístit nastavení doručení nad nabídku. Placená část doporučení nebyla dostupná a netvrdíme, že tento konkrétní postup převádí její obsah.

Tyto zdroje poskytují vodítka pro návrh. Nejsou měřením tohoto webu ani zárukou vyššího počtu objednávek. Vhodnost řešení pro zákazníky pekárny bude potřeba ověřit.

## Ukázkové předpoklady

- Rozhraní je česky, ceny v Kč a PSČ má český pětimístný formát. Konkrétní město ani skutečná oblast rozvozu nejsou určené. PSČ se kontroluje pouze formátem.
- Doručení stojí ukázkově 49 Kč a od hodnoty svačin 350 Kč je zdarma. Produkty, částky i označení cen včetně DPH jsou podklady návrhu k potvrzení pekárnou.
- Nabízejí se tři následující dny pondělí až pátek podle aktuálního data a časového pásma prohlížeče. Výpočet začíná zítřkem, neřeší svátky, uzávěrku objednávek ani kapacitu. Časová okna jsou ukázková. Výběr nic nerezervuje.
- Fotografie jsou generované ilustrační materiály. Receptury a alergeny jsou pracovní údaje, které musí před spuštěním nahradit ověřená data pekárny.
- Účty, skutečné platby a odeslání objednávky nejsou připojené. Platební krok je simulace a závěr výslovně potvrzuje dokončení prototypu.
- Dialog Pobočky zatím neobsahuje smyšlené adresy ani otevírací dobu. Ty doplní pekárna.

## Statický frontend a budoucí backend

Od 9. 10. 2026 jde o čisté Leptos CSR sestavované Trunkem. Axum a serverové vykreslení byly z prototypu odstraněné. GitHub Pages publikuje obsah `dist/`, aplikace používá jednu stránku s kotvami a relativní cesty. Nastavení základní cesty při sestavení umožňuje umístění pod `/brunch/` i na kořeni domény. Registrace, skutečné objednávky a platby potřebují budoucí backend.

## Navázání backendu a ověření

Budoucí backend musí být rozhodujícím zdrojem katalogu, cen, dostupnosti, rozvozových oblastí a termínů. Před přijetím objednávky znovu ověří produkty, počty, dopravu a konečnou částku. Uložené počty v prohlížeči nemohou zaručit skladovou dostupnost ani cenu. Rezervace termínu a platba vyžadují samostatný serverový tok.

Ověřeno 8. 10. 2026 v lokálním prohlížeči na šířkách 320, 390, 768 a 1440 px bez vodorovného přetékání. Prošel filtr kategorií a rostlinných svačin včetně prázdného výsledku, přidávání a odebírání, obnovení košíku po načtení stránky, výpočet 208 + 49 = 257 Kč i doprava zdarma u 416 Kč, změna dne a času bez přepsání aktuálního PSČ, povinné údaje, ukázkové shrnutí a zavření detailu klávesou Escape. Nadpis košíku získá fokus při přechodu mezi kroky. Přidávání množství na mobilu má plochu 44 × 44 px. Žádné chyby JavaScriptu ani hydratace nebyly při těchto kontrolách zaznamenány. Během restartu vývojového serveru se dočasně hlásilo odpojení automatického obnovování.

V původní variantě z 8. 10. 2026 prošlo serverové i WASM sestavení a dva jednotkové testy výpočtů košíku a bezpečného načtení jeho uloženého obsahu. Snímky návrhu jsou v `design/previews`. Fotografie se načítají jako WebP, dohromady 481 542 bajtů. Jde o kontrolu prototypu, ne o měření produkční rychlosti ani plný audit přístupnosti.

## Ověření statického prototypu 9. 10. 2026

Produkční sestavení pro `/` a `/brunch/` prošlo kontrolou souborů a referencí. Celý aktuální balíček má přibližně 1,04 MB před kompresí včetně písem a fotografií. Statický balíček byl načtený v prohlížeči pod `/brunch/` z jednoduchého souborového serveru bez Axumu či API. Načetly se obrázky a styly, loader po spuštění zmizel.

Prošel rostlinný filtr ve sladké kategorii, detail s alergeny, obnovení dvoupoložkového košíku a součet 208 + 49 = 257 Kč. Doručovací krok nepustil prázdná povinná pole, jméno tvořené mezerami ani telefon tvořený písmeny. Vyplnění ukázkových údajů, změna dne a času, návrat pomocí přehledu kroků i opětovné otevření zachovaly rozpracované hodnoty. Dokončeny obě ukázkové platební možnosti. Obnovení stránky zachovalo košík, ale vymazalo jméno a e-mail z formuláře.

Objednávka ověřena na desktopu 1280 px, platební panel na mobilu 390 px a potvrzení i dialog Pobočky na 320 px bez vodorovného přetékání. Zavření klávesou Escape funguje a nadpis získává fokus při změně kroku. Prohlížeč nehlásil chyby JavaScriptu. Snímky jsou `design/previews/checkout-desktop.png` a `design/previews/checkout-mobile.png`. Zkušební košík byl na konci vyprázdněn.

GitHub Actions je připravený, ale vzdálený repozitář nebyl v této pracovní složce připojený a veřejné nasazení nebylo provedeno.
