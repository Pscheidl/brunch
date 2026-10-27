# brunch

Obchod s rozvozem svačin. Frontend používá Rust, Leptos CSR a Trunk. Backend je Rust/Axum s PostgreSQL. Obchod a administraci obsluhuje stejný server, takže nejsou potřeba oddělené domény ani CORS.

## Objednávka a účet

- Dynamický katalog, kategorie, rostlinný filtr, složení a alergeny, dostupné množství a košík zachovaný po obnovení stránky.
- Nabídka zboží a přidávání do košíku jsou dostupné i bez PSČ. Povolené PSČ je nutné až pro dokončení objednávky.
- Nákup bez registrace. Povinné jsou e-mail, ulice, město a PSČ. Jméno, telefon, firma a pokyny pro kurýra jsou volitelné.
- Výběr požadovaného dne a časového okna. Nabízejí se nejbližší tři pracovní dny podle pražského času, bez českých státních svátků.
- Platba kartou přes terminál při doručení, hotově při doručení nebo online přes Teya Hosted Checkout. Online platba se nabízí až po bezpečném nastavení brány podle `docs/teya-payments.md`.
- Server vždy spočítá cenu z databáze. Změnu ceny nebo dopravy mezi výběrem a odesláním musí zákazník znovu potvrdit.
- Objednávka, její položky, odečet skladu a e-mailová fronta se uloží jednou transakcí. Opakovaný požadavek se stejným klíčem nevytvoří další objednávku.
- Registrace a přihlášení jednorázovým odkazem z e-mailu. Platnost 15 minut, potvrzení kliknutím, bez hesel. Účet vzniká až po ověření adresy.
- Přihlášený zákazník vidí své objednávky a může zopakovat nákup za aktuální ceny. Dřívější anonymní objednávky se automaticky nepřiřazují podle shodného e-mailu.
- Potvrzení pro obchod i zákazníka a e-maily při změně stavu objednávky.

## Administrace

Na `/admin/` se správce přihlásí stejným e-mailovým odkazem. Tato verze má jednoho správce určeného `ADMIN_EMAIL` v konfiguraci serveru. Změna adresy a restart převedou oprávnění na nového správce a odeberou je původnímu.

- Zboží: přidání a úpravy, kategorie, složení, alergeny, nahrávání fotografií JPEG/PNG/WebP, dostupnost, archivace a skladové množství. Prázdný sklad znamená neomezenou dostupnost. Fotografie mohou mít nejvýše 2 MB a 4096 pixelů na straně. Server ověří skutečný formát a odstraní vložená metadata.
- Ceny: časová historie každé změny a identifikace správce. Starší objednávky si uchovají původní název, cenu a dopravu.
- Objednávky: hledání a filtrování, detail kontaktu a doručení, poznámka, položky, změny stavu, evidence přijaté platby a historie změn.
- Doprava: cena, volitelná hranice pro dopravu zdarma a e-mail příjemce objednávek.
- Oblasti rozvozu: přidávání a úpravy PSČ s volitelným názvem oblasti, vyhledávání, vypnutí a opětovné zapnutí. Rozvoz je povolen pouze na aktivní PSČ. Všechny bílé znaky se při zadávání ignorují, například `110 00` a `11000` jsou stejné PSČ.
- Zákazníci: ověřené účty a počty jejich objednávek.
- E-maily: přehled fronty, výsledků odeslání a opakování selhaných objednávkových zpráv.

Pro ranní kontrolu otevřete **Objednávky** a zvolte **Dnes**, případně **Zítra** nebo konkrétní **Den doručení**. Filtr vybírá požadovaný den doručení bez ohledu na datum vytvoření objednávky. Lze ho kombinovat se stavem a hledáním, platí i při stránkování. **Všechny dny** zruší pouze omezení na den. Rychlé volby se řídí časovým pásmem Europe/Prague. Pro kontrolu objednávek určených k přípravě sledujte jejich stav, zejména nevyřizujte objednávky čekající na online platbu, zrušené nebo vypršelé.

Oznámení o nové objednávce chodí na adresu **Doprava a nastavení → E-mail pro příjem objednávek**. U platby při doručení vznikne hned po přijetí objednávky, u online platby až po ověřené úhradě. Zákazník dostává vlastní potvrzení, před online úhradou nejprve zprávu o čekání na platbu. E-maily obsahují den a časové okno doručení, položky, množství, ceny, způsob platby, adresu a kontakty. Objednávky zůstávají dostupné v administraci i při potížích s doručením zprávy.

Platba při doručení přejde rovnou do stavu „Přijata“. Online objednávka nejprve čeká na zaplacení a přijme se až po ověření úhrady u Teya. Dále stavy postupují `přijata → potvrzena → příprava → na cestě → doručena`. Před doručením lze nezaplacenou objednávku zrušit, čímž se rezervované kusy jednou vrátí. Platbu při doručení lze zaznamenat na cestě nebo při doručení. Online platbu ověřuje server, ručně ji označit za přijatou nelze. Nezaplacenou online objednávku lze stornovat až po ověření konečného neúspěchu nebo expirace relace u Teya. Doručená objednávka musí být zaplacená. Zrušení zaplacené objednávky vyžaduje budoucí samostatný proces vrácení peněz.

## Místní spuštění

Potřebujete Rust podle `rust-toolchain.toml`, Trunk 0.21.14 a PostgreSQL 18. Pro místní databázi a zachytávání e-mailů je připraven Docker Compose:

```bash
cp .env.example .env
docker compose up -d
./scripts/dev.sh
```

Obchod a administrace běží na `http://127.0.0.1:3100`, místní e-mailová schránka na `http://127.0.0.1:18026`. Všechny zprávy z výchozího místního nastavení zůstávají v Mailpitu. Správce má adresu `admin@example.test`, objednávky chodí na `orders@example.test`.

Při prvním spuštění se automaticky provedou migrace, založí správce a vloží původních šest ukázkových produktů. Výchozí doprava stojí 49 Kč, od 350 Kč je zdarma. Existující data se při dalších startech nepřepisují. Ukázkové zboží, složení a fotografie je před skutečným prodejem potřeba upravit v administraci.

Před první objednávkou nastavte v administraci v sekci **Oblasti rozvozu** skutečně obsluhovaná PSČ. Migrace žádná PSČ nepovoluje automaticky. Prázdný seznam nebo seznam bez aktivních PSČ znamená, že nové objednávky nelze přijmout. Zákazník uvidí dostupnost při zadávání adresy a server ji znovu ověří při odeslání. Změna oblastí neovlivní dříve přijaté objednávky ani jejich bezpečné opakování při ztrátě odpovědi.

Pro nativní PostgreSQL nastavte `DATABASE_URL`, pro vlastní místní SMTP odpovídající proměnné. Backend čte `.env` v kořeni projektu. `./scripts/dev.sh` sestaví frontend a spustí backend. Po změně frontendu skript znovu spusťte. Statický výstup musí být pro tento backend sestaven s cestou `/`.

```bash
./scripts/build.sh --public-url /
cargo run --locked --manifest-path server/Cargo.toml
```

Skripty hledají Trunk v `PATH`, `target/tools/trunk/trunk` nebo v sousedním `obecni-web/target/tools/trunk/trunk`. Lze nastavit `BRUNCH_TRUNK_BIN`. Sousední projekt není závislost aplikace. Sestavení přesune zaváděcí JavaScript do samostatného souboru, aby bylo kompatibilní s bezpečnostní politikou serveru.

`docker compose down` zastaví místní služby a zachová databázový svazek. Přepínač `-v` by databázi smazal.

## Testy

Použijte samostatnou místní databázi. Integrační testy vytvářejí a odstraňují vlastní schémata, nepřepisují aplikační tabulky.

```bash
export TEST_DATABASE_URL=postgres://brunch:brunch-local@127.0.0.1:55434/brunch
./scripts/test.sh
```

Samostatné příkazy:

```bash
cargo test --locked --lib
cargo test --locked --manifest-path server/Cargo.toml --lib
cargo test --locked --manifest-path server/Cargo.toml --test api --test mail --test security --test media --test payments -- --ignored --test-threads=1
cargo clippy --locked --manifest-path server/Cargo.toml --all-targets -- -D warnings
```

Databázové a SMTP integrační testy jsou označeny `ignored`, protože bez PostgreSQL nemají smysluplný výsledek. Skript výše je vždy výslovně spouští a při chybě nebo chybějící databázi skončí neúspěšně. SMTP test spouští skutečný místní testovací SMTP server a ověřuje i opakování po výpadku. Scénáře a rozsah jsou v `docs/test-plan.md`, výsledky místního ověření v `docs/local-verification.md`.

## Provozní nastavení

`PUBLIC_URL` musí odpovídat adrese webu. Mimo localhost je vyžadováno HTTPS. Relace používá HttpOnly cookie, SameSite=Lax a při HTTPS také Secure. Zápisové API ověřuje původ požadavku a vlastní hlavičku. Administrátorská oprávnění se kontrolují na serveru pro každý endpoint, ne pouze zobrazením menu.

Pro doručování mimo počítač nastavte skutečné `SMTP_HOST`, `SMTP_PORT`, `SMTP_FROM`, `SMTP_USERNAME`, `SMTP_PASSWORD`, `SMTP_TLS=true`, `ORDER_EMAIL` a `ADMIN_EMAIL`. SMTP používá povinný STARTTLS. Tajné hodnoty patří do prostředí nebo `.env`, nikdy do Git repozitáře. Služba potřebuje zálohovaný PostgreSQL, HTTPS reverse proxy a provozní dohled. API dostupnost ověřuje `/api/health`.

Fronta opakuje selhané odeslání s prodlužujícím se odstupem, po osmi pokusech zůstane zpráva jako selhaná v administraci. E-mail se považuje za odeslaný až po přijetí SMTP serverem, nikoli za prokazatelně doručený do schránky. Při pádu přesně mezi SMTP přijetím a uložením výsledku může přijít duplikát. Opakované odeslání má stejné Message-ID. Objednávka se při potížích s e-mailem neztratí.

Časová okna jsou požadavkem zákazníka, nikoli rezervací kapacity kurýra. Oblast rozvozu se kontroluje podle PSČ, existence konkrétní ulice nebo shoda města s PSČ se neověřuje. Tato verze nemá napojení na dopravce, GPS sledování ani automatické plánování tras. Obsluha potvrzuje přijetí a proveditelnost doručení v administraci. Veřejné spuštění vyžaduje doplnění skutečných kontaktních a obchodních údajů provozovatele, podmínek a pravidel zpracování zákaznických údajů.

## Nasazení na Zoner Cloud

Pro samostatný VPS je připravené produkční nasazení přes Docker Compose s PostgreSQL a Caddy pro automatické HTTPS. Hotový aplikační obraz se sestavuje mimo VPS pomocí `./scripts/package-release.sh`. Doména, SMTP, správce, příjemce objednávek a volitelné Teya platby se nastavují v oddělené chráněné konfiguraci. Staging a produkce automaticky nezakládají ukázkové zboží. Přípravu serveru, všechny potřebné údaje, kontroly, zálohy, zkoušku obnovy a aktualizaci popisuje [docs/zoner-cloud.md](docs/zoner-cloud.md). GitHub Pages neumí spustit tento backend ani PostgreSQL.

## Teya a obnova plateb

Nastavení tajného klíče v souboru, veřejného podpisového klíče, webhooku a chování při restartu popisuje [docs/teya-payments.md](docs/teya-payments.md). Platební úloha se ukládá s objednávkou do PostgreSQL. Po restartu pokračuje se stejným identifikátorem. Přijetí platby vyžaduje ověření u Teya, návrat zákazníka z brány nestačí. Nezaplacená online objednávka po 60 minutách automaticky vyprší a vrátí rezervované zboží. Lhůtu lze nastavit proměnnou `ONLINE_PAYMENT_TIMEOUT_MINUTES` (1 až 1440), uložený termín se restartem nemění. Údržba termíny kontroluje každou sekundu i při výpadku Teya. Známé platby server sám pravidelně ověřuje, pozdní úhradu eviduje k dořešení obsluhou. Online platby zůstávají bez konfigurace vypnuté. Místní simulované testy nenahrazují ověření se skutečným staging účtem Teya před spuštěním.

## Struktura

- `src/`, `style/`: zákaznický obchod a objednávka
- `public/admin/`: administrace
- `server/src/`: autentizace, obchodní API, e-mailová fronta
- `server/migrations/`: verzované PostgreSQL migrace
- `server/tests/`: skutečné HTTP, databázové a SMTP testy
- `docs/api-contract.md`: datové rozhraní
- `compose.yaml`, `.env.example`: místní služby a konfigurace
