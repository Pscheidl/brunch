# Ověření objednávek a administrace

## Automatické testy

HTTP integrační testy běží proti skutečné PostgreSQL. Každý test vytváří vlastní náhodně pojmenované schéma a po úspěchu ho odstraní. Použijte výhradně místní testovací databázi. Uživatel databáze potřebuje oprávnění vytvářet schémata. Testy se nespouštějí tiše bez databáze, jsou výslovně označené `ignored` a při explicitním spuštění bez `TEST_DATABASE_URL` skončí chybou.

```bash
cd server
TEST_DATABASE_URL=postgres://brunch:brunch-local@127.0.0.1:55434/brunch cargo test --test api --test mail --test security --test media -- --ignored
```

Při selhání testu může zůstat izolované schéma `brunch_test_<uuid>` pro diagnostiku. Nikdy nespouštějte testy proti produkční databázi.

| Oblast | Co ověřuje HTTP test |
| --- | --- |
| Nákup bez účtu | Stačí e-mail a adresa, poznámka se uloží, účet se nevytváří, částky počítá server |
| Opakovaný požadavek | Stejný klíč a obsah vrátí stejnou objednávku, jiný obsah skončí konfliktem, e-maily se nezdvojí |
| Souběh | Ze dvou požadavků na poslední kus uspěje jeden, dva souběžné požadavky se stejným klíčem vytvoří jednu objednávku |
| Cena a historie | Změna ceny má audit, původní objednávka uchová název a cenu, zastaralý součet ani zastaralá verze se nepřepíší |
| Fotografie | Nahrání a načtení fotografie, skutečný formát obrázku, limit velikosti a rozměrů, odmítnutí SVG a poškozených dat, odstranění metadat |
| Sklad a storno | Objednávka rezervuje kusy, storno je vrátí právě jednou, konečný stav nelze znovu otevřít |
| Registrace | Účet vznikne až ověřením e-mailu, odkaz je jednorázový i při souběhu, prošlý odkaz nefunguje |
| Relace a oprávnění | Odhlášení a expirace zneplatní relaci, všechny administrační cesty vyžadují správce, požadavky z cizího původu jsou odmítnuty |
| Soukromí | Zákazník nevidí cizí objednávky, starší nákup hosta se nepřipojí podle pouhého e-mailu, odpovědi používají `no-store`, seznam e-mailů neodhaluje přihlašovací odkazy |
| Doprava | Nová cena dopravy, vypnutý a dosažený limit dopravy zdarma, změna provozního e-mailu |
| Oblasti rozvozu | Správa pouze pro správce, normalizace všech bílých znaků, unikátní PSČ i při souběhu, konflikt starší verze, úprava a vypnutí/zapnutí oblasti, veřejný seznam pouze aktivních PSČ |
| Kontrola PSČ objednávky | Prázdný seznam a nepovolené PSČ odmítnou nákup bez změny skladu či e-mailů, povolené PSČ projde, opakování přijaté objednávky funguje i po vypnutí oblasti a změně mezer |
| Objednávky | Postupné stavy, oddělená platba kartou/hotovostí při doručení, audit změn, zakázané storno zaplacené objednávky, filtrování |
| E-mailová fronta | Potvrzení zákazníkovi a provozovateli ve stejné transakci s objednávkou, chyba při ukládání druhého e-mailu vrátí i objednávku a sklad, opakování selhané zprávy |
| Neplatná data | Chybné kontakty, prázdná adresa, datum mimo nabídku, chybná platba a množství neuloží objednávku ani e-mail |

## Ruční ověření rozhraní

Nejprve v administraci povolit testovací PSČ `110 00`. V poli i při hledání vyzkoušet mezery, tabulátor, nezalomitelnou mezeru a jejich kombinace. Duplikát se nesmí založit. Ověřit dostupnost v dialogu adresy i objednávky, vypnutí oblasti před konečným odesláním a zobrazení chyby bez vzniku objednávky. Po testu vrátit původní nastavení oblastí.

1. V anonymním okně přidat zboží, změnit množství a odeslat nákup pouze s adresou a e-mailem. Vyplnit například patro, zvonek a pokyn k předání. Ověřit zobrazený součet a potvrzení.
2. Zkontrolovat obě zprávy v místním poštovním serveru. Vypnout SMTP a ověřit, že objednávka zůstane přijatá a e-mail čeká na opakované odeslání.
3. Vyžádat přihlašovací odkaz, otevřít ho, zkontrolovat účet, odhlásit se a zkusit stejný odkaz podruhé.
4. Přihlásit správce a projít všechny záložky v administraci. Založit produkt včetně fotografie, složení a alergenů, změnit cenu, zobrazit historii, vyprodat a archivovat produkt.
5. V administraci vyhledat objednávku, zkontrolovat adresu a pokyny, otevřít mapu a vytisknout rozvozový list. Postupně potvrdit přípravu, rozvoz a doručení. Označit převzetí platby. U jiné nezaplacené objednávky ověřit storno.
6. Otevřít stejný produkt nebo objednávku ve dvou oknech. Po uložení prvního musí druhé okno zobrazit konflikt, nikoli přepsat novější změny.
7. Změnit dopravu a limit zdarma, ověřit součet nové objednávky. Dřívější objednávka musí zachovat původní částky.
8. Vyzkoušet ovládání klávesnicí, zavírání dialogu klávesou Escape, chybové a prázdné stavy. Zkontrolovat mobilní šířku 390 px i desktop. Tabulky mohou mít vlastní vodorovný posuv.

Rezervace skutečné rozvozové kapacity není součástí této verze. Online platby Teya mají samostatné scénáře níže. Časové okno je požadavek zákazníka, který provozovatel potvrdí.

## Opakovatelné ověření administrace v prohlížeči

Skript `scripts/test-admin-browser.cjs` používá Playwright a Chromium proti běžícímu místnímu serveru a Mailpitu. Přihlásí správce skutečným e-mailovým odkazem, nahraje fotografii, vytvoří a upraví vlastní testovací produkt, ověří historii, uloží nezměněné nastavení dopravy a projde objednávkou až do doručení včetně platby. Zkontroluje odkaz do mapy, telefon, tisk a šířku 390 px. Vlastní produkt na konci archivuje. Testovací objednávku ponechá pro kontrolu. Výchozí produkty ani cenu dopravy nemění.

```bash
TEST_BASE_URL=http://127.0.0.1:3100 \
TEST_MAILPIT_URL=http://127.0.0.1:18026 \
TEST_ADMIN_EMAIL=admin@example.test \
node scripts/test-admin-browser.cjs
```

Prostředí musí mít dostupné balíčky Playwright a prohlížeč Chromium. Snímky a jednostránkový rozvozový list se ukládají do `output/admin-browser/`. Při chybě vznikne také `failure.png`.

Ověřeno 21. 10. 2026: tento scénář úspěšně prošel proti místnímu serveru, PostgreSQL a Mailpitu. V prohlížeči nebyla zachycena žádná neošetřená chyba. Zkontrolovány byly také snímky desktopu, mobilní šířky 390 px a jednostránkový tisk A4.

## Prohlížečové testy

Vyžadují běžící místní obchod se vzorovým katalogem, aktivním testovacím PSČ `11000`, Mailpit, Node.js a Playwright 1.62.1. Pro test obnovy spojení je potřeba také klient `psql` a URL stejné místní databáze, kterou používá server. Skripty odmítnou veřejnou adresu webu. Vytvářejí skutečné testovací objednávky v místním obchodě, nepoužívejte je na produkčních datech. Integrační HTTP testy si povolené PSČ zakládají samy pouze ve svých izolovaných schématech.

```bash
npm install
npx playwright install chromium
npm run test:storefront
npm run test:admin
BRUNCH_TEST_DATABASE_URL=postgres://brunch:brunch-local@127.0.0.1:55434/brunch npm run test:recovery
```

- Obchod: nákup hosta, obě platby při doručení, skutečný potvrzovací e-mail, registrace odkazem, oddělení historie hosta, nákup s účtem, opakování nákupu, košík po obnovení, odhlášení a mobilní rozměry.
- Administrace: přihlášení, nahrání fotografie, vytvoření a archivace zboží, změna a historie ceny, uložení dopravy, hledání objednávky, mapa, tisk rozvozového listu, postupné doručení a přijetí platby, zákazníci a pošta, mobilní zobrazení.
- Obnova: výpadek katalogu, ztracená odpověď po skutečně uložené objednávce a bezpečné opakování, skutečný konflikt ceny 409 a nové potvrzení.

Samostatný `npm run test:postal` vyžaduje místní databázi bez aktivních oblastí rozvozu. Přihlásí správce, přidá vlastní testovací PSČ a ověří zadávání i vkládání Unicode whitespace, hledání, duplicity, editaci a vypnutí/zapnutí. Nabídka i přidávání do košíku musí fungovat bez zadaného PSČ a bez aktivních oblastí. Na šířce 390 px test projde oběma zákaznickými formuláři, ověří odmítnutí objednávky po vypnutí oblasti během nákupu a úspěšné opakování po zapnutí. Na konci vlastní objednávku stornuje a oblast pozastaví. Existující aktivní oblasti test sám nemění, při jejich přítomnosti skončí s vysvětlením. Snímky jsou v `output/postal-browser/`.

Snímky a místní výsledky jsou v ignorované složce `output/`. `BRUNCH_PSQL_BIN` umožňuje zadat vlastní cestu k psql. Databázové kontroly ověřují jeden skutečně uložený záznam po opakování požadavku.

Další integrační testy ověřují ochranu poštovní fronty proti pozdnímu výsledku starého pracovníka, odstranění prošlých přihlašovacích odkazů, okamžité odebrání práv původnímu správci, úplný rollback při selhání vložení e-mailu a bezpečné nahrání obrázků včetně formátu, velikosti, rozměrů a orientace fotografie.

## Online platby Teya

`server/tests/payments.rs` používá izolovaná PostgreSQL schémata, skutečné HTTP a místní simulátor Teya. Ověřuje ztrátu odpovědi při založení relace, opakování stejného klíče i těla po restartu, obnovu přes API při chybějícím webhooku, podpis původních bajtů, nesoulad částky/měny/provozovny/reference/relace/typu, opakovaná oznámení, ochranu oprávnění a přechodů objednávky. Testy jsou součástí `scripts/test.sh` i backendového CI.

`scripts/test-payment-browser.cjs` spouští skutečnou sestavenou WASM aplikaci s řízenými odpověďmi API a vlastním místním webovým serverem. Ověřuje dostupnost online metody, uchování objednávky před přesměrováním, ztracenou odpověď, návrat bez falešného potvrzení, obnovení stránky, výpadek stavového API a omezení administrace. Vyžaduje sestavený `dist`, Node.js, Playwright a Chromium. Výsledky zapisuje do `output/browser/payment-results.json`.

```bash
TEST_DATABASE_URL=postgres://user@127.0.0.1:5432/test_db cargo test --locked --manifest-path server/Cargo.toml --test payments -- --ignored --test-threads=1
npm run test:payments
```

Před produkcí je nutné opakovat úspěch, odmítnutí, restart a webhook přes skutečný Teya staging účet a veřejné HTTPS. Lokální simulátor neověřuje aktivaci obchodníka ani skutečné karetní platby.

### Odmítnutí, expirace a opožděná úhrada

Simulátor rozlišuje zamítnutý pokus v aktivní relaci od konečného odmítnutí a od síťového výpadku. Testy ověřují zachování rezervace při pokračující relaci, úspěch dalšího pokusu bez druhé relace, konečné odmítnutí a expiraci bez falešného přijetí. Storno vrací sklad právě jednou, pozdní úhrada uzavřenou objednávku neotevře a vytvoří jedinečný doklad i upozornění.

Lhůta objednávky se testuje posunem uloženého termínu v izolované databázi, bez čekání hodinu. Kontroly zahrnují výchozích 60 minut, jinou konfiguraci, neměnnost po restartu, okamžik před a po termínu, vypnutou konfiguraci Teya, již zaplacené objednávky a platby při doručení, souběžné pracovníky, selhání ukládání e-mailu a opožděný úspěch. Prohlížeč ověřuje konečný stav, skrytí platebního odkazu, ukončení automatického dotazování, nové ID nové objednávky a zamčené ovládání expirované objednávky ve správě.

Samostatný test drží databázový zámek, nechá na něm čekat potvrzení platby a během čekání posune termín. Ověřuje skutečný čas po získání zámku, uzavření objednávky a evidenci pozdní úhrady bez přijetí k vyřízení.

Další test pozastaví již načtenou úlohu při OAuth požadavku, během čekání doručí platný webhook s dosud neznámou relací a nechá uplynout lhůtu. Starší rozhodnutí pracovníka nesmí ztratit již potvrzené oznámení. Původní relace se ověří, úhrada se uloží a nová platba se nezaloží.

Doklad úhrady se porovnává s uloženým záznamem v PostgreSQL a znovu načítá po restartu. Nepřihlášený návštěvník i zákazník mají administrační detail zakázaný, veřejné vytvoření, opakování objednávky, zákaznická historie ani stavové API nevracejí identifikátory transakce a relace. Prohlížečové scénáře ověřují částku, měnu, oba identifikátory a čas ověření v detailu zaplacené i expirované objednávky s pozdní platbou.
