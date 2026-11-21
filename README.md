# Brunch Era

Obchod s rozvozem svačin. Frontend používá Rust, Leptos CSR a Trunk. Backend je Rust/Axum s PostgreSQL. Obchod a administraci obsluhuje stejný server, takže nejsou potřeba oddělené domény ani CORS.

## Objednávka a účet

- Dynamický katalog, kategorie, rostlinný filtr, složení a alergeny, dostupné množství a košík zachovaný po obnovení stránky.
- Nabídka zboží a přidávání do košíku jsou dostupné i bez PSČ. Povolené PSČ je nutné až pro dokončení objednávky.
- Nákup bez registrace. Povinné jsou jméno příjemce, telefon, e-mail a doručovací adresa. Návrhy adres z RÚIAN doplní ulici, město a PSČ. Všechna pole lze vyplnit i ručně. Firma a pokyny pro kurýra jsou volitelné.
- Kalendář umožňuje vybrat kterýkoli pracovní den od nejbližšího následujícího pracovního dne podle pražského času. Víkendy a české svátky jsou uzavřené. V administraci lze nastavit výchozí časová okna rozvozu a přepsat je u jednotlivých PSČ.
- Platba kartou přes terminál při doručení, hotově při doručení nebo online přes Teya Hosted Checkout. Online platba se nabízí až po bezpečném nastavení brány podle `docs/teya-payments.md`.
- Server vždy spočítá cenu z databáze. Změnu ceny nebo dopravy mezi výběrem a odesláním musí zákazník znovu potvrdit.
- Objednávka, její položky, odečet skladu a e-mailová fronta se uloží jednou transakcí. Opakovaný požadavek se stejným klíčem nevytvoří další objednávku.
- Registrace a přihlášení jednorázovým odkazem z e-mailu. Platnost 15 minut, potvrzení kliknutím, bez hesel. Účet vzniká až po ověření adresy.
- Přihlášený zákazník vidí své objednávky a může zopakovat nákup za aktuální ceny. Dřívější anonymní objednávky se automaticky nepřiřazují podle shodného e-mailu.
- Potvrzení pro obchod i zákazníka a e-maily při změně stavu objednávky.

## Administrace

Na `/admin/` se správce přihlásí e-mailem a heslem. Účty a oprávnění jsou pouze v databázi, start aplikace je nepřepisuje podle konfigurace. Hesla se ukládají pomocí Argon2id s vlastním náhodným saltem pro každé heslo. Zákaznický e-mailový odkaz nikdy nepřihlásí správce do administrace.

Sekce **Administrátoři** umožňuje přidat správce podle e-mailu a hesla, nastavit nové heslo a odebrat oprávnění. Posledního správce nelze odebrat ani při souběžných požadavcích. Změna hesla nebo odebrání oprávnění okamžitě ukončí jeho relace. Zákaznický účet a objednávky se nemažou. Novému správci předejte přístupové údaje bezpečně, aplikace hesla neposílá e-mailem.

- **Zboží a ceny** umožňuje správu nabídky, skladu, složení, alergenů i archivaci. Minimální odběr je 0 až 99 kusů. Při hodnotě 0 přidá zákazník první kus, při vyšším minimu rovnou nastavené množství. Prázdná hodnota skladu znamená neomezenou dostupnost.
- K produktu lze nahrát JPEG, PNG nebo WebP do 40 MiB. Server upraví orientaci a zmenší snímek na nejvýše 1 440 px na delší straně. Neprůhledné fotografie uloží jako JPEG v kvalitě 88, průhledné jako PNG. Změny cen mají historii. Starší objednávky si uchovají původní název, cenu a dopravu.
- V sekci **Kategorie** lze založit nebo přejmenovat kategorii i odstranit kategorii se zbožím. Přejmenování se promítne do přiřazených položek. Po odstranění zůstanou všechny položky v katalogu bez kategorie.
- V sekci **Objednávky** lze vyhledávat a filtrovat, zobrazit kontakt a doručení, evidovat úhradu a měnit stav. Zaškrtnutím více objednávek ve stejném stavu je přesunete společně. Při hromadném dokončení dobírek potvrďte převzetí plateb. Pokud některou z vybraných objednávek mezitím někdo změnil, obnovte výběr.
- Před vystavením první faktury vyplňte sekci **Fakturační údaje**. V detailu objednávky doplňte odběratele, datum plnění a splatnost. Plátce DPH zadá sazbu u každé položky a účtované dopravy. Vystavenou fakturu s vlastním číslem můžete kdykoli znovu otevřít, vytisknout nebo uložit jako PDF přes tiskový dialog.
- V sekci **Doprava a nastavení** nastavíte cenu dopravy, hranici dopravy zdarma, příjemce objednávek a výchozí časová okna. V **Oblastech rozvozu** lze obsluhovaná PSČ přidávat, měnit a vypínat. U jednotlivého PSČ můžete výchozí okna nahradit vlastními. Mezery v PSČ se ignorují.
- **Zákazníci** zobrazí ověřené účty a počty jejich objednávek. V sekci **E-maily** uvidíte frontu a stav odeslání. Tlačítko **Napsat hromadný e-mail** zobrazí počet různých adres z objednávek včetně nákupů bez registrace. Rozeslání potvrďte po vyplnění předmětu a textu. Každému příjemci přijde samostatná zpráva.

Pro ranní kontrolu otevřete **Objednávky** a zvolte **Dnes**, případně **Zítra** nebo konkrétní **Den doručení**. Filtr vybírá požadovaný den doručení bez ohledu na datum vytvoření objednávky. Lze ho kombinovat se stavem a hledáním, platí i při stránkování. **Všechny dny** zruší pouze omezení na den. Rychlé volby se řídí časovým pásmem Europe/Prague. Pro kontrolu objednávek určených k přípravě sledujte jejich stav, zejména nevyřizujte objednávky čekající na online platbu, zrušené nebo vypršelé.

Oznámení o nové objednávce chodí na adresu **Doprava a nastavení → E-mail pro příjem objednávek**. U platby při doručení vznikne hned po přijetí objednávky, u online platby až po ověřené úhradě. Zákazník dostává vlastní potvrzení, před online úhradou nejprve zprávu o čekání na platbu. E-maily obsahují den a časové okno doručení, položky, množství, ceny, způsob platby, adresu a kontakty. Objednávky zůstávají dostupné v administraci i při potížích s doručením zprávy.

Platba při doručení přejde rovnou do stavu „Přijata“. Online objednávka nejprve čeká na zaplacení a přijme se až po ověření úhrady u Teya. Dále stavy postupují `přijata → potvrzena → na cestě → doručena`. Před doručením lze nezaplacenou objednávku zrušit, čímž se rezervované kusy jednou vrátí. Platbu při doručení lze zaznamenat na cestě nebo při doručení. Online platbu ověřuje server, ručně ji označit za přijatou nelze. Nezaplacenou online objednávku lze stornovat až po ověření konečného neúspěchu nebo expirace relace u Teya. Doručená objednávka musí být zaplacená. Zrušení zaplacené objednávky vyžaduje budoucí samostatný proces vrácení peněz.

## Místní spuštění

Potřebujete Rust podle `rust-toolchain.toml`, Trunk 0.21.14 a PostgreSQL 18. Pro místní databázi a zachytávání e-mailů je připraven Docker Compose:

```bash
cp .env.example .env
docker compose up -d
./scripts/dev.sh
```

Obchod a administrace běží na `http://127.0.0.1:3100`, místní e-mailová schránka na `http://127.0.0.1:18026`. Všechny zprávy z výchozího místního nastavení zůstávají v Mailpitu, objednávky chodí na `orders@example.test`. Správce se automaticky nezakládá.

Při prvním spuštění se automaticky provedou migrace a ve vývojovém režimu vloží původních šest ukázkových produktů. Prvního správce vytvořte jednorázovým příkazem do databáze. Heslo zadáváte skrytě a předává se standardním vstupem, ne argumentem ani proměnnou prostředí:

```bash
python3 -c 'import getpass,sys; sys.stdout.write(getpass.getpass("Heslo prvního správce: "))' | cargo run --locked --manifest-path server/Cargo.toml -- --initialize-admin admin@example.test
```

Příkaz odmítne založení, pokud už existuje správce. Další účty se spravují v administraci. Výchozí doprava stojí 49 Kč, od 350 Kč je zdarma. Existující data se při dalších startech nepřepisují. Ukázkové zboží, složení a fotografie je před skutečným prodejem potřeba upravit v administraci.

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

Nové objednávky ukládají IP adresu spojení. Je dostupná pouze v detailu objednávky správci, starší objednávky mají prázdnou hodnotu. Výchozí `TRUST_CADDY_CLIENT_IP=false` používá adresu TCP spojení a ignoruje hlavičky klienta. Produkční Compose zapíná tuto volbu pouze za vyhrazeným Caddy, který přepisuje `X-Brunch-Client-IP` skutečnou adresou svého TCP spojení. Aplikační port nesmí být publikován ani jinak dostupný mimo soukromou Docker síť. `X-Forwarded-For`, `Forwarded` a `X-Real-IP` se pro ukládání nepoužívají. Při prvním nasazení této změny nejprve načtěte nový Caddyfile, pak spusťte novou aplikaci. Při přidání další proxy nebo CDN je nutné znovu nastavit důvěryhodný vstup.

Obchodní podmínky jsou na `/obchodni-podminky/` a upravují se v administraci v sekci **Obchodní podmínky**. Uložení rovnou zveřejní novou verzi bez restartu. Nová objednávka vyžaduje výslovný souhlas s aktuální verzí, uchová její číslo a zákazník dostane celé přijaté znění e-mailem. Starší verze zůstávají dostupné přes `?version=N` a úprava podmínek nemění již přijaté objednávky. Výchozí text se zavádí pouze jednou, restart ani nové nasazení administrátorské změny nepřepíší.

Provozovatel zadal název Mysterium Gaudi, dočasné IČO `11111111` a pozdější doplnění sídla a kontaktů. Tyto údaje jsou v prvním oddílu transparentně označeny a je potřeba je nahradit skutečnými údaji v administraci. Právní podklady a provozní předpoklady jsou v [docs/terms-sources.md](docs/terms-sources.md).

Pro nasazení pomocí Docker Compose nastavte v `/etc/brunch/brunch.env` server `SMTP_HOST`, port `SMTP_PORT` (výchozí 587) a odesílatele `SMTP_FROM`. Pokud poskytovatel vyžaduje přihlášení, nastavte `SMTP_USERNAME` a buď `SMTP_PASSWORD`, nebo cestu `SMTP_PASSWORD_FILE`. Produkční spojení používá STARTTLS. `ORDER_EMAIL` určuje počátečního příjemce objednávek. Tajné hodnoty patří do prostředí nebo chráněných souborů, nikdy do Git repozitáře. Administrátorské účty se v konfiguraci nenastavují. Služba potřebuje zálohovaný PostgreSQL, HTTPS reverse proxy a provozní dohled. API dostupnost ověřuje `/api/health`.

Fronta opakuje selhané odeslání s prodlužujícím se odstupem, po osmi pokusech zůstane zpráva jako selhaná v administraci. E-mail se považuje za odeslaný až po přijetí SMTP serverem, nikoli za prokazatelně doručený do schránky. Při pádu přesně mezi SMTP přijetím a uložením výsledku může přijít duplikát. Opakované odeslání má stejné Message-ID. Objednávka se při potížích s e-mailem neztratí.

Časová okna jsou požadavkem zákazníka, nikoli rezervací kapacity kurýra. Oblast rozvozu se kontroluje podle PSČ, existence konkrétní ulice nebo shoda města s PSČ se neověřuje. Tato verze nemá napojení na dopravce, GPS sledování ani automatické plánování tras. Obsluha potvrzuje přijetí a proveditelnost doručení v administraci. Veřejné spuštění vyžaduje doplnění skutečných kontaktních a obchodních údajů provozovatele, podmínek a pravidel zpracování zákaznických údajů.

## Nasazení na Zoner Cloud

Pro samostatný VPS je připravené produkční nasazení přes Docker Compose s PostgreSQL a Caddy pro automatické HTTPS. Hotový aplikační obraz se sestavuje mimo VPS pomocí `./scripts/package-release.sh`. Doména, SMTP a volitelné Teya platby se nastavují v chráněné konfiguraci na VPS. Účty správců a provozní nastavení obchodu jsou v databázi. Staging a produkce automaticky nezakládají ukázkové zboží. Přípravu serveru, všechny potřebné údaje, kontroly, zálohy, zkoušku obnovy a aktualizaci popisuje [docs/zoner-cloud.md](docs/zoner-cloud.md). GitHub Pages neumí spustit tento backend ani PostgreSQL.

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
