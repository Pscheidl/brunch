# Zákaznické funkce a provozní přehled

Rozšíření z 21. listopadu 2026 doplňuje běžný nákup, zákaznický účet, společné objednávky a administraci. Součástí nejsou opakované pravidelné objednávky ani sběr zájmu o rozvoz mimo podporovanou oblast.

## Nabídka a dokončení nákupu

- Hledání v nabídce toleruje diakritiku a okolní mezery. Oblíbené produkty a košík se uchovávají v daném prohlížeči.
- Kartu produktu lze otevřít přímým odkazem `/?product=ID`. Detail ukazuje složení, čitelný význam čísel alergenů, případný minimální odběr a výběr varianty.
- Množství lze zadat přímo. Minimum, dostupné kusy a limit 99 kusů kontroluje rozhraní i server. U minimálního odběru je vidět také cena tohoto množství.
- Dostupnost rozvozu lze ověřit podle PSČ už před výběrem. Zobrazují se dostupné termíny, cena dopravy a částka zbývající do dopravy zdarma.
- Košík nabízí vhodné doplnění a rychlý výběr stejné svačiny pro 5, 10 nebo 20 lidí. Počet a obsah lze před odesláním upravit.
- Rozepsané doručovací a fakturační údaje se obnovují v dané relaci prohlížeče. Po úspěšném nákupu se koncept odstraní. Před odesláním zákazník vidí adresu, položky, varianty, platbu a konečný součet.
- Fakturační údaje může zadat také nepřihlášený zákazník. Ukládají se s konkrétní objednávkou v jedné transakci.
- Opakované odeslání stejného požadavku vrací stejnou objednávku. Změněný obsah se stejným klíčem vrací konflikt. Ceny, sklad a termín znovu kontroluje server.

Varianty jednoho produktu sdílejí cenu i sklad. Košík podporuje jednu variantu daného produktu. Cenově odlišné výrobky nebo souběžně objednatelné samostatné varianty založte jako samostatné produkty. U produktu s variantami je výběr povinný a uchovává se v položce objednávky i jejím názvu. Historické objednávky se změnou katalogu nepřepisují.

## Zákaznický účet

Přihlášení jednorázovým odkazem z e-mailu zpřístupní stránku `/customer/`. Samotné zadání e-mailu do objednávky přístup k účtu neuděluje.

- Přehled nejbližších doručení zobrazuje stav objednávky. V aktivním okně se obnovuje po 30 sekundách, pokud zákazník právě nevyplňuje formulář nebo nemá otevřený dialog.
- Historii lze hledat podle čísla, adresy nebo firmy a procházet po 20 objednávkách. U starší objednávky lze otevřít doklady a požádat o změnu.
- Adresář umožňuje uložit nejvýše 20 pojmenovaných míst doručení, upravit je, odebrat nebo použít při dalším nákupu.
- Výchozí fakturační údaje lze uložit do účtu a přenést do objednávky. Údaje konkrétní objednávky lze upravit do vystavení faktury.
- Vystavená faktura je dostupná pouze přihlášenému vlastníkovi objednávky a správci. Tiskové zobrazení umožňuje tisk nebo uložení PDF prostřednictvím prohlížeče. Změna adresáře či výchozích údajů vystavený doklad nepřepisuje.
- Zákazník může odeslat žádost o změnu nebo zrušení dosud neuzavřené objednávky. U jedné objednávky může čekat jedna žádost. Opakování po výpadku nevytvoří druhou žádost ani druhé upozornění.

Žádost sama nemění stav objednávky, cenu ani platbu. Obsluha požadovanou úpravu nejprve vyřeší v příslušné části administrace a poté v sekci **Požadavky zákazníků** zapíše odpověď a žádost označí jako vyřízenou nebo zamítnutou. Zákazník dostane odpověď e-mailem. Storno a případné vrácení platby podléhají stávajícímu postupu obsluhy.

## Společné objednávky

Na `/team/` vytvoří přihlášený organizátor košík a nastaví uzávěrku v českém čase. Odkaz pro kolegy umožní vložit jméno a vlastní výběr bez založení účtu. Každý účastník upravuje svůj výběr, organizátor vidí všechny a může je upravit do uzavření košíku.

- Jeden organizátor může mít nejvýše 10 otevřených košíků. Košík podporuje 50 účastníků a 50 různých produktů, každý v celkovém množství nejvýše 99 kusů.
- Uzávěrku lze založit 5 minut až 31 dní dopředu. Po uzávěrce či ručním uzavření již účastníci výběr nemění. Organizátor může dosud neobjednaný košík znovu otevřít s novou uzávěrkou.
- Minimální odběr se ověřuje za celý tým při uzavření. Rozdílné varianty stejného produktu musí organizátor před uzavřením sjednotit nebo použít samostatné produkty.
- Uzavřený košík organizátor přenese do běžného nákupu, zkontroluje obsah a zadá doručení i platbu. Objednávka se odesílá pod jeho přihlášeným e-mailem.
- Přenos nerezervuje sklad a nevytváří objednávku. Aktuální cenu a dostupnost ověří až dokončení nákupu. Verze košíku brání odeslání zastaralého výběru po opětovném otevření a úpravě.
- Jeden týmový košík lze spojit s jednou objednávkou. Souběžné odeslání z více oken nevytvoří více objednávek. Do jednoho běžného košíku lze přenést nejvýše jeden týmový košík.

Pozvánku posílejte pouze zamýšleným účastníkům. Oprávnění upravit vlastní výběr se ukládá v prohlížeči účastníka. Po smazání těchto dat může výběr upravit organizátor. Pozvánka nezpřístupňuje doručovací údaje, faktury ani objednávkovou historii.

## Administrace a obsah, který je třeba doplnit

V sekci **Zboží** funguje hledání bez závislosti na diakritice a filtr kategorie. Formulář produktu vybírá z vytvořených kategorií a umožňuje zadat varianty po jednom řádku. Kategorie se spravují samostatně. U zboží jsou vidět upozornění na chybějící složení a alergeny.

Nastavení vzhledu webu nově obsahuje veřejný kontaktní e-mail, telefon, název provozovatele, adresu, IČO a stručný popis oblasti rozvozu. Prázdné kontaktní údaje se návštěvníkům nevykreslují. Popis oblasti nenahrazuje seznam povolených PSČ. Veřejný kontakt a neveřejní příjemci objednávek mají samostatné nastavení.

Pro dokončení obsahu chybějí ověřené kontaktní údaje a ověřené alergeny koláčků. Provozovatel je doplní podle skutečných podkladů. Prázdné alergeny aplikace označuje jako nedoplněné, nikoliv jako výrobek bez alergenů. U jednotlivých variant je potřeba udržovat odpovídající složení a alergeny v popisu produktu, případně je rozdělit do samostatných výrobků.

Požadované časové okno potvrzuje obsluha. Tato změna nepřidává rezervaci kapacity rozvozu ani automatické pravidelné objednávky. Skutečné doručení e-mailů závisí na provozní konfiguraci a běhu stávající poštovní fronty.

## Nasazení a ověření

Backend vyžaduje migrace `0021_customer_experience.sql`, `0022_team_orders.sql`, `0023_public_contacts.sql` a `0024_catalog_choices.sql`. S backendem nasaďte sestavený obchod, nové stránky `public/customer/` a `public/team/` i aktualizovanou administraci. Před produkčním nasazením dodržte postup projektu pro zálohu a migrace.

HTTP testy vytvářejí vlastní izolovaná schémata v místní testovací PostgreSQL. `TEST_DATABASE_URL` musí odkazovat na testovací databázi. Žádný z těchto testů nespouští odesílání do skutečných schránek.

```bash
cargo test --locked --manifest-path server/Cargo.toml --test customer_experience --test team_orders -- --ignored --test-threads=1
```

Zákaznická sada ověřuje oddělení účtů, přístup hosta, CSRF, verzování adres a údajů, neměnnost vystaveného dokladu, idempotentní žádosti a atomický zápis oznámení. Řízené souběhy ověřují také checkout a zákaznickou žádost během zápisu administrátorské historie, aby nevzniklo databázové uváznutí. Týmová sada kontroluje vlastnictví, oddělení účastníků, termíny, souběh výběrů, sklad, minima, varianty, uzavření, revize a ochranu před dvojím objednáním.

Prohlížečové scénáře jsou v `scripts/test-customer-browser.cjs`, `scripts/test-team-browser.cjs`, `scripts/test-customer-ux-browser.cjs` a `scripts/test-admin-catalog-ux.cjs`. Používají místní web a řízené odpovědi API. Zákaznická stránka je ověřená v šířkách 1440, 390 a 320 px včetně neúspěšného mazání adresy, opakování požadavku, vypršení relace a tiskového zobrazení faktury. Výstupní snímky patří do ignorované složky `output/`.
