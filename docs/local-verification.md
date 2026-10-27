# Den doručení a objednávkové e-maily: místní ověření 24. 10. 2026

Administrace nově filtruje objednávky podle požadovaného dne doručení. Nabízí konkrétní datum a volby Dnes, Zítra a Všechny dny. Filtr lze kombinovat se stavem i hledáním, zůstává zachovaný při stránkování, obnovení a navigaci. Dny se počítají v Europe/Prague.

- HTTP integrační sada: 12 testů prošlo. Nové testy ověřují 54 objednávek napříč dvěma dny, kombinace filtrů, stránkování, odlišení od data vytvoření, neplatná data a ochranu přístupu.
- SMTP: 1 test prošel včetně síťového výpadku a obnovy pracovní fronty.
- Platby: 25 testů prošlo. Doplněné kontroly ověřují příjemce a obsah e-mailů před úhradou a po ní, včetně termínu, položek a částky. Stejné obsahové kontroly prošly v HTTP testu platby při doručení.
- Backendové jednotkové testy: 16 prošlo. Formátování, Clippy se zákazem varování, release sestavení a kontrola veřejných souborů prošly.
- Prohlížečové scénáře filtru: 5 prošlo. Kombinování filtrů, stránkování, navigace, chyba API a obnova, pražská půlnoc, změna času a roku při jiném pásmu prohlížeče, šířka 390 px bez přetečení celé stránky. Žádné neošetřené chyby. Desktopové a mobilní snímky byly vizuálně zkontrolovány.

Výsledky a snímky jsou v `output/order-dates-browser/`. Testy běžely v izolovaných schématech dočasné místní PostgreSQL na portu 58434. Skutečná konfigurace `.env` nebyla čtena ani měněna. Ověřeno je místní SMTP, nikoli doručení přes externí server do skutečné schránky. Nedošlo k nasazení ani pushi.

# Online platby: místní ověření 24. 10. 2026

Integrace Teya Hosted Checkout je implementovaná lokálně. Bez přístupových údajů je online metoda vypnutá. Neproběhl push ani nasazení. Produkční konfigurace a skutečný účet Teya nebyly použity.

## Ověřený rozsah

- Frontendové jednotkové testy: 5 prošlo.
- Backendové jednotkové testy: 16 prošlo, včetně přesné domény přesměrování, zákazu veřejně čitelného souboru tajného klíče, zákazu jeho umístění ve veřejných souborech webu a validace nastavitelné lhůty objednávky.
- Dosavadní PostgreSQL/HTTP/SMTP sady: 17 prošlo.
- Nové testy plateb: 25 prošlo. Používají PostgreSQL 18.6 v izolovaných schématech a místní HTTP simulátor Teya. Zkoušejí obnovu se stejným klíčem po ztracené odpovědi a novém startu aplikace, výpadek webhooku, RSA podpis přes původní tělo, nesoulad údajů, souběžné pracovníky, cizí relaci podepsanou společným klíčem a opakované transakce. Zpracovávaná platba nevrací URL k novému otevření brány. Vynucené selhání druhého potvrzovacího e-mailu vrátí úhradu, objednávku, historii i první zprávu. Další pokus pak vše přijme právě jednou.
- Platební sada dále pokrývá odmítnutí, síťový výpadek, nastavitelnou expiraci, vrácení skladu právě jednou, pozdní úhradu a upozornění, souběh expirace s potvrzením a překročení termínu během čekání na databázový zámek. Přijatý webhook neztratí práci ani při současném rozhodnutí starého pracovníka o ruční kontrole. Doklad odpovídá uloženému záznamu v databázi a zůstává dostupný správci po restartu. Nepřihlášený návštěvník ani zákazník nemají přístup k administračnímu detailu, identifikátory dokladu neunikají do veřejných odpovědí.
- Prohlížečové testy skutečného WASM frontendu s řízenými odpověďmi API: 16 prošlo. Online metoda bez konfigurace skrytá, uchování původní objednávky před přesměrováním, ztracená odpověď se stejným klíčem, návratová URL bez důkazu zaplacení, obnovení stránky, výpadek API, stav vyžadující kontrolu a omezení administrace včetně ověřeného storna. Expirace skryje platební odkaz a zastaví dotazování, nová objednávka dostane nový klíč. Administrace zobrazuje doklad uhrazené i pozdě uhrazené expirované objednávky, včetně částky, měny, identifikátorů Teya a času ověření.
- Optimalizované sestavení frontendu, kontrola veřejných souborů, formátování a kontrola změn prošly. Backendový Clippy se zákazem varování prošel.

Databáze byla spuštěná pouze místně na portu 58434 z existujících PostgreSQL binárních souborů s daty v `/tmp/brunch-payments-runtime`. Testy nepřepisovaly dosavadní aplikační databázi. Výsledky prohlížeče jsou v `output/browser/payment-results.json`.

## Zbývající vnější ověření

Skutečná transakce v Teya stagingu, aktivace provozovny, konkrétní staging URL a doručení webhooku přes produkční HTTPS proxy nejsou místním simulátorem ověřené. Získání údajů a postup spuštění popisuje [teya-payments.md](teya-payments.md). Aplikace nepovažuje samotný návrat z brány ani podpis webhooku bez ověření relace za přijetí platby. Doba uchování idempotentních požadavků Teya není ve veřejné dokumentaci stanovená, nejasný starý výsledek se proto zastaví k prověření obsluhou.

Níže zůstává původní záznam ověření před přidáním online plateb.

# Místní ověření 21. 10. 2026

Výsledek: kontroly správy PSČ a navazující kontroly objednávek prošly. Rozšíření navazuje na původní backend v commitu `6627d19`. Neproběhl push ani nasazení na vzdálený server.

## Prostředí

- Rust 1.98.1, Leptos 0.8.21, Trunk 0.21.14.
- Skutečný PostgreSQL 18.6 na `127.0.0.1:55434`.
- Místní Mailpit na SMTP portu 11026 a webu `http://127.0.0.1:18026`.
- Chromium přes Playwright 1.62.1, desktop 1440 px a mobil 390 px.
- Obchod a administrace na `http://127.0.0.1:3100`.

PostgreSQL pro toto ověření byl spuštěn nativně z dočasného adresáře `/tmp/brunch-runtime`, systémové služby se neměnily. Docker Compose je připraven pro opakovatelné vlastní spuštění, jeho běh zde ověřen nebyl. Účty a objednávky v místním náhledu jsou testovací, adresy používají doménu `example.test`. Zprávy zachycuje Mailpit a neodesílají se skutečným zákazníkům.

## Automatické kontroly

Příkaz pro opakování kompletní sady:

```bash
TEST_DATABASE_URL=postgres://brunch:brunch-local@127.0.0.1:55434/brunch CARGO_NET_OFFLINE=true ./scripts/test.sh
```

| Sada | Výsledek |
| --- | --- |
| Frontendové testy košíku, dopravy a PSČ | 5 prošlo |
| Backendové jednotkové testy | 8 prošlo |
| HTTP integrační scénáře na PostgreSQL | 10 prošlo |
| Skutečné SMTP, výpadek a opakování | 1 prošel |
| Bezpečnost relací, práv a poštovní fronty | 4 prošly |
| Nahrávání a poskytování obrázků | 2 prošly |
| Celkem | 30 prošlo, 0 selhalo, 0 přeskočeno v explicitně spuštěných sadách |
| Formátování, backendový Clippy se zákazem varování | Prošlo |
| Optimalizované WebAssembly sestavení a kontrola souborů | Prošlo |
| Syntaxe administrace a kontrola změn | Prošlo |

Při sestavení frontendových závislostí Rust upozorňuje na budoucí nekompatibilitu balíčku `proc-macro-error2 2.0.1`. Aktuální připnutý toolchain sestavení dokončuje úspěšně.

## Skutečné prohlížečové ověření

Po rozšíření o PSČ znovu prošly skripty `test-storefront-browser.cjs` a `test-checkout-recovery.cjs`. Nový `scripts/test-postal-browser.cjs` ověřil také:

- Zobrazení katalogu a přidání do košíku bez PSČ i při prázdném seznamu aktivních oblastí.
- Správu oblastí, normalizaci Unicode whitespace při zadávání i vložení, vyhledání, odmítnutí duplicity, úpravu a pozastavení/zapnutí.
- Oba zákaznické formuláře, blokování nepodporovaného PSČ a přijetí povoleného PSČ s bílými znaky.
- Vypnutí oblasti po zobrazení závěrečné kontroly. Server vrátil 400, nevytvořil objednávku ani zákaznický e-mail.
- Zapnutí oblasti a opakování se stejným klíčem vytvořilo právě jednu objednávku. Test ji následně stornoval.
- Mobilní šířku 390 px a nepřítomnost neošetřených výjimek prohlížeče. Snímky byly vizuálně zkontrolovány.

Testovací oblast byla po ověření odstraněna. Pro ruční místní ukázku je povoleno PSČ `11000` s názvem „Místní ukázka“. Tato hodnota je pouze v místní databázi, migrace ani seed aplikace žádná PSČ automaticky nepovolují.

`scripts/test-storefront-browser.cjs` prošel celý nákup bez účtu s hotovostí při doručení, doručení potvrzení do Mailpitu, registraci a přihlášení odkazem, nákup s účtem a kartou při doručení, vlastní historii, opakování objednávky, zachování košíku po obnovení stránky a odhlášení. Obchod ani objednávkový dialog nepřetékají při šířce 390 px. Žádná neošetřená chyba JavaScriptu.

`scripts/test-checkout-recovery.cjs` ověřil tři situace:

1. Katalogové API nebylo dostupné. Stránka zobrazila chybu a po výslovném opakování načetla skutečné produkty.
2. Server objednávku uložil, ale prohlížeč její odpověď ztratil. Opakování poslalo stejný obsah a klíč a vrátilo stejné ID. Přímá SQL kontrola našla právě jednu objednávku.
3. Prohlížeč zobrazil starší cenu. Server vrátil skutečný konflikt 409. Stránka obnovila nabídku a až další výslovné potvrzení vytvořilo objednávku za aktuální cenu. V databázi zůstal jeden záznam.

Při původním ověření backendu `scripts/test-admin-browser.cjs` ověřil skutečné přihlášení správce, nahrání a načtení fotografie, vytvoření produktu, změnu a historii ceny, archivaci, nastavení dopravy, hledání objednávky, adresu a pokyny, odkazy do mapy a telefonu, tisk rozvozového listu na jednu stranu A4, postupné změny až do doručení s potvrzením platby, zákazníky, e-mailovou frontu a mobilní administraci. Žádná neošetřená chyba JavaScriptu. Testovací produkt je po ověření archivovaný.

Snímky obrazovky a testovací PDF jsou v ignorovaných složkách `output/browser/`, `output/admin-browser/` a `output/postal-browser/`. Nejsou součástí veřejného sestavení.

## Hranice ověření

Ověřeny jsou místní aplikace, PostgreSQL a SMTP. Produkční SMTP, HTTPS reverse proxy, skutečný platební terminál a provoz kurýrů nebyly připojeny. Zvolený termín je požadovaný čas doručení potvrzovaný obsluhou, nikoli rezervace rozvozové kapacity. Online platební brána zůstává podle zadání na pozdější verzi.

# Příprava nasazení 27. 10. 2026

Pro přípravu Zoner VPS prošly následující kontroly:

| Kontrola | Výsledek |
| --- | --- |
| Backendové jednotkové testy | 16 prošlo |
| Konfigurace v oddělených procesech bez externích spojení | 5 prošlo |
| PostgreSQL HTTP, pošta, obrázky, platby a bezpečnost | 45 prošlo |
| Balení vydání, vyloučení tajných souborů a neměnný vstup z commitu | 7 prošlo |
| Kontroly nasazení, záloh, zkoušky obnovy a veřejných otisků | 18 prošlo |
| Optimalizované sestavení backendu a frontendu | Prošlo |
| Kontrola statických souborů, Rust formátování a Clippy | Prošlo |
| Načtení produkčního souboru skutečným Docker Compose | Prošlo, bez Docker daemonu |
| Ověření syntaxe a konfigurace Caddy | Prošlo, bez získání veřejného certifikátu |

Databázové testy použily novou dočasnou PostgreSQL 18.6. Samostatná zkouška nativního release backendu dále provedla skutečný `deploy/init-db.sh`, ověřila aplikační roli bez práv superuživatele, všechny migrace a prázdný produkční katalog. Každý soubor frontendu byl načten přes HTTP a porovnán podle SHA-256. Proběhla skutečná záloha `pg_dump`, obnova do oddělené databáze a kontrola platebních tabulek i poštovní fronty. SIGTERM ukončil aplikaci s návratovým kódem 0, po novém startu zůstala uložená nastavení zachována. Dočasné databáze a aplikace byly po zkouškách zastaveny.

Testy balení a wrapperu používají náhradní Docker příkaz. Ověřují chování skriptů a chybové situace, nikoli skutečný kontejner. V tomto prostředí není Docker daemon, proto **sestavení Docker obrazu a běh celé Compose sestavy nebyly provedeny**. Připravený workflow `.github/workflows/deployment.yml` je ověří po spuštění CI pomocí `scripts/test-deployment-smoke.sh`. Workflow v této přípravě spuštěn nebyl.

Nevzniklo připojení k VPS ani skutečné SMTP nebo Teya. Veřejné DNS, HTTPS, doručení zpráv, skutečná staging platba a zálohy mimo server se ověří až po dodání údajů. Příprava nemění historické výsledky výše a nenahrazuje tyto provozní zkoušky.
