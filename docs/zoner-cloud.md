# Nasazení Brunch Era na Zoner Cloud

Projekt je připraven pro samostatný Linux VPS s Docker Engine a Docker Compose. Aplikace se sestaví na pracovním počítači nebo v CI, na VPS se přenese hotový obraz. Tento dokument je postup pro pozdější nasazení, ne záznam o provedeném nasazení. Skutečná doména, přístupy, pošta a účet Teya se doplní až podle údajů provozovatele.

## Velikost a rozdělení služeb

Pro menší místní rozvoz je výchozí konfigurace 2 vCPU, 2 GB RAM a 40 GB SSD. Pro větší paměťovou rezervu lze zvolit 4 GB RAM. Jde o odhad podle architektury, nikoli o ověřený limit souběžných objednávek.

Na serveru běží tři služby podle [deploy/compose.yaml](../deploy/compose.yaml):

| Služba | Úloha | Přístup z internetu |
| --- | --- | --- |
| Caddy | HTTPS, obnova certifikátu a předávání požadavků aplikaci | TCP 80 a 443 |
| Brunch Era | Obchod, administrace, e-mailová fronta a platby | Pouze přes Caddy |
| PostgreSQL 18 | Objednávky, zákazníci, fotografie, platby a poštovní fronta | Bez zveřejněného portu |

Mailpit je pouze pro místní vývoj. Produkční pošta používá externí SMTP. Runtime aplikace běží pod UID/GID `10001:10001`. Aplikační obraz neobsahuje přístupové údaje ani Rust sestavovací nástroje.

## Údaje potřebné před nasazením

| Oblast | Co dodat nebo rozhodnout |
| --- | --- |
| VPS | IP adresa, operační systém, přístup přes SSH klíč a oprávnění správce |
| Doména | Hlavní doména obchodu, přístup k DNS, případné přesměrování dalších názvů |
| HTTPS | E-mail pro správu certifikátu `ACME_EMAIL` |
| Správce | E-mail jako login a heslo prvního účtu, jednorázově založeného v databázi |
| Objednávky | Adresa `ORDER_EMAIL` pro první nastavení příjemce objednávek |
| Pošta | `SMTP_HOST`, port se STARTTLS, uživatel, heslo a povolený odesílatel `SMTP_FROM` |
| Teya | Zvlášť staging a produkce: `store_id`, `client_id`, `client_secret` a RSA veřejný klíč webhooků |
| Zálohy | Cílové úložiště mimo VPS, přístup, četnost kopírování, uchování a příjemce upozornění |
| Provoz obchodu | Skutečné zboží, ceny, alergeny, sklad, rozvozová PSČ, doprava, kontakty a identifikační údaje provozovatele |

Přístupové údaje nepatří do Git repozitáře, release archivu, veřejných souborů ani do sdílených výpisů příkazů. Provozní konfigurace se ukládá samostatně na VPS do `/etc/brunch/brunch.env` s oprávněním `0600`.

## Příprava VPS

Vhodný systém je Debian 13 nebo Ubuntu 26.04 LTS na architektuře x86_64. Docker Engine, Buildx a Compose plugin instalujte podle oficiálního postupu pro [Debian](https://docs.docker.com/engine/install/debian/) nebo [Ubuntu](https://docs.docker.com/engine/install/ubuntu/). Na VPS není potřeba instalovat Rust, Trunk, Node ani samostatný PostgreSQL mimo kontejnery.

Pokud ještě chybí doména a skutečná pošta, použijte oddělený [neveřejný náhled přes SSH](private-preview.md). Tento režim má vlastní databázi a zachytává poštu pouze místně. Veřejnou produkční konfiguraci ani její validaci nenahrazuje.

Povolte SSH pro správu a veřejné TCP porty 80 a 443. Porty databáze a aplikace nejsou publikované. Při nastavení firewallu zohledněte, že publikované porty Dockeru mohou obcházet běžná pravidla UFW, jak popisuje stejný instalační návod. SSH přístup zachovejte před změnou firewallu.

DNS záznam A hlavní domény musí směřovat na IPv4 tohoto VPS. AAAA přidávejte pouze při funkčním IPv6 směrování na stejnou instalaci. Caddy potřebuje dostupné porty a správné DNS pro získání certifikátu. Doména musí být holý název, například `obchod.vase-domena.cz`, bez `https://`, cesty a koncového lomítka.

### Dočasný veřejný přístup přes IPv4

Pokud DNS zatím nelze přesměrovat, lze obchod provozovat přímo přes veřejnou IPv4 s důvěryhodným HTTPS. V provozním souboru nastavte `DOMAIN` na IPv4 VPS, `ALLOW_PUBLIC_IP=true` a `ACME_PROFILE=shortlived`. Výchozí `ALLOW_PUBLIC_IP=false` nadále vyžaduje DNS jméno. Soukromé, rezervované a dokumentační adresy ani IPv6 tento režim nepovoluje. SMTP a ostatní produkční požadavky zůstávají povinné.

[Let's Encrypt vydává certifikáty pro IP adresy](https://letsencrypt.org/2026/01/15/6day-and-ip-general-availability) s platností 160 hodin. Caddy používá explicitní veřejný ACME issuer, profil `shortlived` a automatickou obnovu. Port 80 musí být dostupný pro HTTP challenge, port 443 pro obchod. Svazek `caddy_data` musí zůstat zachovaný, obsahuje účet a certifikáty. HTTP se automaticky přesměruje na HTTPS. Nastavení `default_sni` zajišťuje výběr certifikátu také pro prohlížeče, které u IP adres neposílají SNI.

Po změně DNS na tento VPS přepněte `DOMAIN` na hlavní doménu, `ALLOW_PUBLIC_IP=false` a `ACME_PROFILE=classic`. Znovu spusťte `brunch.sh up`, ověřte certifikát, přihlášení a veřejné soubory. Dříve odeslané přihlašovací odkazy používají původní adresu, proto si vyžádejte nový odkaz. Údaje a svazek databáze se při změně adresy nemění.

Na VPS připravte adresáře:

```bash
sudo install -d -m 0755 /opt/brunch/releases /etc/brunch
sudo install -d -m 0711 /etc/brunch/secrets
sudo install -d -m 0700 /var/backups/brunch
```

Adresář tajných souborů musí aplikaci dovolit průchod. Samotné soubory budou čitelné pouze UID 10001. Zálohy obsahují zákaznické údaje a musí zůstat soukromé.

## Sestavení a předání vydání

Na pracovním počítači nebo CI s Dockerem spusťte z kořene čistého, commitnutého projektu:

```bash
./scripts/package-release.sh
```

Skript vytváří obraz pro `linux/amd64` a samostatný release archiv. Výpis uvede umístění výsledku a přesný tag obrazu. Výchozí kontrola odmítá necommitnuté změny. Přepínač `--allow-dirty` slouží pouze k místnímu ověření a vytváří zřetelně označené testovací vydání.

Výstup je ve výchozím adresáři `output/releases`: `brunch-<commit12>.tar.gz` a `brunch-<commit12>.tar.gz.sha256`. Jiný výstupní adresář lze předat jako argument skriptu. Uvnitř archivu je stejnojmenný adresář s `image.tar.gz`, `release.env`, `SHA256SUMS`, `build-inputs.sha256`, adresářem `deploy` a dokumentací. Provozní `/etc/brunch/brunch.env`, tajné klíče, databázová data ani zálohy se do něj nepřidávají. `release.env` obsahuje metadata vydání, nikoli skutečnou provozní konfiguraci. `build-inputs.sha256` zachycuje otisky zdrojů použitých při sestavení, samotné zdroje do release archivu nepřidává.

Před prvním přenosem zkontrolujte obsah archivu a jeho kontrolní součet. Po přenosu ověřte součet celého archivu ještě před rozbalením. Soubor `.sha256` očekává archiv vedle sebe. Hodnotu porovnejte také s originálem na počítači, kde vydání vzniklo. Na VPS rozbalujte pod `/opt/brunch/releases` do nového adresáře `brunch-<commit12>`, nikoli přes existující vydání. V rozbaleném adresáři ověřte obsah dříve, než nahrajete obraz:

```bash
sha256sum --check SHA256SUMS
sudo docker image load --input image.tar.gz
```

Samotný seznam součtů uvnitř archivu neověřuje původ vydání. Po načtení porovnejte výstup `docker image inspect --format '{{.Id}}' brunch:<commit12>` s hodnotou `IMAGE_ID` v `release.env`.

Z `release.env` přečtěte hodnotu `BRUNCH_IMAGE` a přeneste ji do provozního nastavení. Soubor není potřeba spouštět příkazem `source`. Provozní soubor se při aktualizaci nikdy nenahrazuje souborem z archivu.

### Varianta bez Dockeru na pracovním počítači

Pokud pracovní počítač nemá Docker, lze Rust a frontend zkompilovat lokálně a na VPS pouze sestavit malý runtime obraz podle stejného principu jako obecní web:

```bash
python3 scripts/package-prebuilt.py
```

Skript vyžaduje čistý commit, Rust podle `rust-toolchain.toml`, Trunk 0.21.14, wasm-bindgen 0.2.129 a `readelf`. Čte soubory z konkrétních Git objektů, nikoli z průběžně měněné pracovní kopie. Ověřuje architekturu x86_64, maximální požadavek glibc 2.36 a knihovny dostupné v runtime. Výsledek `brunch-prebuilt-<commit12>.tar.gz` obsahuje `runtime/`, provozní skripty, kontrolní součty a záznam verzí nástrojů. Neobsahuje `.env`, klíče ani databázi. Toto je jiný formát než hotový obraz z `package-release.sh`, nemá `image.tar.gz` ani `IMAGE_ID`.

Po přenosu ověřte vnější součet, rozbalte do nového adresáře a ověřte `SHA256SUMS`. Z jeho `release.env` přečtěte přesný commit a tag. Následující zástupné hodnoty nahraďte těmito ověřenými údaji:

```bash
sudo docker build --platform linux/amd64 --build-arg SOURCE_REVISION=CELÝ_COMMIT --tag brunch:COMMIT12 runtime
sudo docker image inspect --format '{{.Id}}' brunch:COMMIT12
```

Docker na VPS instaluje pouze základní runtime balíčky a přidá hotový binární soubor a statické soubory. Rust ani WASM na VPS nekompiluje. Výsledné ID obrazu zaznamenejte do záznamu nasazení, nastavte `BRUNCH_IMAGE` a pokračujte kontrolou konfigurace, spuštěním a ověřením veřejných souborů stejně jako u plného release archivu. Základní Debian obraz a balíčky se stahují při sestavení, proto samotný commit negarantuje stejné ID obrazu při pozdějším opakování. Pro přesný návrat uchovejte již sestavený obraz.

## Provozní nastavení

Při první instalaci zkopírujte [deploy/.env.example](../deploy/.env.example) do `/etc/brunch/brunch.env` a nastavte oprávnění `0600`. Kopii provádějte pouze pokud tam konfigurace ještě není:

```bash
sudo install -m 0600 deploy/.env.example /etc/brunch/brunch.env
sudoedit /etc/brunch/brunch.env
```

Všechny příklady v šabloně je třeba nahradit. Validace odmítá ukázkové adresy pro staging a produkci. Soubor má syntaxi Docker Compose `.env`, není určený ke spuštění shellem. Hodnoty s `$`, `#` nebo mezerami zapisujte podle pravidel Compose, nejlépe v jednoduchých uvozovkách. Hesla nevypisujte do příkazové řádky ani do diagnostických výpisů `docker compose config`.

| Proměnná | Význam |
| --- | --- |
| `BRUNCH_IMAGE` | Přesný tag hotového obrazu z tohoto vydání |
| `APP_ENV` | `production` pro zákazníky, `staging` pro oddělené ověření |
| `COMPOSE_PROJECT_NAME` | Stabilní název instalace, například `brunch-production` |
| `DOMAIN` | Hlavní DNS jméno, případně výslovně povolená veřejná IPv4. Compose odvodí `PUBLIC_URL=https://...` |
| `ALLOW_PUBLIC_IP` | Výchozí `false`, dočasný přístup přes veřejnou IPv4 povolí přesné `true` |
| `ACME_PROFILE` | `classic` pro doménu, pro IPv4 povinně `shortlived` |
| `ACME_EMAIL` | Skutečný e-mail pro certifikát HTTPS |
| `POSTGRES_ADMIN_PASSWORD` | Náhodné heslo správce databáze `postgres`, přesně 64 hexadecimálních znaků |
| `POSTGRES_PASSWORD` | Jiné náhodné heslo aplikačního uživatele `brunch`, přesně 64 hexadecimálních znaků |
| `ORDER_EMAIL` | Výchozí příjemce objednávek při založení databáze |
| `SMTP_HOST`, `SMTP_PORT` | Externí SMTP, obvykle port `587` |
| `SMTP_TLS` | Compose jej nastavuje napevno na `true`, používá STARTTLS |
| `SMTP_FROM` | Odesílatel povolený poskytovatelem SMTP |
| `SMTP_USERNAME`, `SMTP_PASSWORD` | Přihlášení k SMTP, vyplňují se společně |
| `SMTP_PASSWORD_FILE` | Volitelná cesta k souboru místo `SMTP_PASSWORD` |
| `ONLINE_PAYMENT_TIMEOUT_MINUTES` | Lhůta nových nezaplacených online objednávek, 1 až 1440 minut, výchozí 60 |
| `SECRETS_DIR` | Hostitelský adresář tajných souborů, výchozí `/etc/brunch/secrets` |

`ORDER_EMAIL` nepřepisuje již uložené nastavení obchodu. Po prvním spuštění hlavního příjemce kontrolujte v **Doprava a nastavení → Hlavní e-mail pro příjem objednávek**. Další adresy přidejte do pole **Další příjemci objednávek**, celkem lze nastavit až 20 různých příjemců. Uložení adres v administraci nevyžaduje restart. Poplatky za dopravu a hranici dopravy zdarma upravte tamtéž. Správci jsou výhradně v databázi, žádná proměnná konfigurace je nezakládá ani nepřepisuje.

### První administrátor a přechod z přihlašování e-mailem

Administrace používá login ve formě e-mailové adresy a heslo o délce 12 až 128 znaků. Heslo je v databázi uložené jako Argon2id s vlastním náhodným saltem. Parametry 19 MiB, dvě iterace a jedno vlákno odpovídají [doporučení OWASP](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html). Hesla ani hashe se neposílají e-mailem a nevypisují v administračním API.

Migrace 0011 ukončí staré administrátorské relace a odebere původní role bez hesla. Zákaznické účty a objednávky zachová. Před prvním nasazením této verze vytvořte zálohu, zastavte starou aplikaci a pod novou verzí jednorázově založte původnímu provozovateli účet s novým heslem. Databáze přitom musí běžet. Z provozního souboru odstraňte staré `ADMIN_EMAIL`.

Na serveru po přepnutí `/opt/brunch/current` a tagu obrazu:

```bash
sudo -v
python3 -c 'import getpass,sys; sys.stdout.write(getpass.getpass("Heslo prvního správce: "))' | sudo -n /opt/brunch/current/deploy/brunch.sh initialize-admin spravce@vase-domena.cz
sudo /opt/brunch/current/deploy/brunch.sh up
```

Příkaz předá heslo pouze standardním vstupem. Nezadávejte je do argumentů, proměnných prostředí, repozitáře ani historie shellu. Inicializace je povolena jen pokud neexistuje žádný administrátor. Další účty i nová hesla spravujte v sekci **Administrátoři**. Změna hesla a odebrání role zneplatní všechny relace daného správce. Posledního správce nelze odebrat.

Po této migraci nepouštějte nad stejnou databází starou verzi s e-mailovým přihlášením správce. Starý kód nemá nové oddělení relací a může přepsat oprávnění. Případná obnova musí zahrnout odpovídající databázovou zálohu a znovu provést bezpečný přechod na hesla. Běžný restart nové verze zachová účty i oprávnění beze změny.

Dvě různá hesla databáze vytvořte například správcem hesel nebo dvěma voláními `openssl rand -hex 32` v bezpečném prostředí. `POSTGRES_ADMIN_PASSWORD` patří správci databáze, `POSTGRES_PASSWORD` aplikačnímu účtu bez práv superuživatele. Při prvním startu se založí databáze `brunch` a její vlastník `brunch`. Compose sestaví `DATABASE_URL` automaticky z aplikačního hesla, adresu ručně nezadávejte. U existujícího datového svazku změna proměnných sama nezmění hesla uživatelů PostgreSQL. Pozdější rotace vyžaduje koordinovanou změnu v databázi i aplikační konfiguraci.

SMTP v této verzi používá STARTTLS. Samotné nastavení portu 465 nezapne implicitní TLS. Doména odesílatele musí mít DNS záznamy požadované poskytovatelem, zejména SPF a DKIM. Před prodejem ověřte skutečné doručení do schránky správce, obchodu a zákazníka.

Pro tajný SMTP soubor používejte například hostitelskou cestu `/etc/brunch/secrets/smtp-password`. Nastavte vlastníka `10001:10001` a oprávnění `0600`. V konfiguraci pak bude `SMTP_PASSWORD_FILE=/run/brunch-secrets/smtp-password` a `SMTP_PASSWORD` zůstane prázdné. Aplikace samostatně podporuje také `DATABASE_URL_FILE`, ale dodané Compose nasazení používá výše popsanou automaticky sestavenou adresu. Jedna hodnota se nesmí zároveň zadat přímo i souborem.

## Teya Hosted Checkout

Bez `TEYA_ENVIRONMENT` jsou online platby vypnuté. Platba hotově nebo terminálem při doručení může fungovat samostatně. Dokumentace integrace, ověřování webhooku a obnovy rozpracovaných plateb je v [teya-payments.md](teya-payments.md).

Pro aktivaci doplňte:

```dotenv
TEYA_ENVIRONMENT=staging
TEYA_STORE_ID=skutecny-identifikator-provozovny
TEYA_CLIENT_ID=skutecny-identifikator-klienta
TEYA_CLIENT_SECRET_FILE=/run/brunch-secrets/teya-client-secret
TEYA_WEBHOOK_PUBLIC_KEY_FILE=/run/brunch-secrets/teya-webhook-public.pem
TEYA_WEBHOOK_PREVIOUS_PUBLIC_KEY_FILE=
```

Tento příklad patří k instalaci s `APP_ENV=staging`. Produkce vyžaduje `APP_ENV=production` a `TEYA_ENVIRONMENT=production`. Kombinaci staging aplikace a produkčních plateb nebo opačně validace odmítá.

Soubor `teya-client-secret` obsahuje samotné tajemství klienta, bez názvu proměnné a bez uvozovek. Na hostiteli patří do `/etc/brunch/secrets/teya-client-secret`, s vlastníkem `10001:10001` a oprávněním `0600` nebo `0400`. Do kontejneru se tento adresář připojuje pouze pro čtení jako `/run/brunch-secrets`.

`teya-webhook-public.pem` je RSA **veřejný** klíč od Teya ve formátu `BEGIN PUBLIC KEY`, alespoň 2048 bitů. Nejde o privátní klíč obchodníka. Při rotaci lze dočasně dodat i předchozí veřejný klíč. Po každé změně klíče nebo obsahu tajného souboru použijte `sudo /opt/brunch/current/deploy/brunch.sh restart`. Tento příkaz po kontrole znovu vytvoří aplikační kontejner, aby načetl nové soubory, a provede zdravotní kontrolu. Samotná změna souboru bez restartu jeho obsah v paměti aplikace neaktualizuje.

V portálu Teya nastavte událost `payment.succeeded.v1` na adresu:

```text
https://VAŠE_DOMÉNA/api/payments/teya/webhook
```

Webhook nesmí blokovat přihlašování, HTML výzva proti robotům nebo přepis těla požadavku. Caddy jej předává aplikaci stejně jako ostatní API. Přijetí platby vyžaduje ověření u Teya, pouhý návrat zákazníka na web nestačí.

Staging a produkce musí mít jiné domény, konfigurace, přístupové údaje a databázové svazky. Změna `COMPOSE_PROJECT_NAME` vytváří odlišné prostředky Compose. S dodaným nastavením obě instalace potřebují vlastní VPS nebo IP a odpovídající předávání portů. Dvě současné instance na jednom hostiteli se stejnými porty 80 a 443 nelze spustit pouze změnou názvu projektu. Společná HTTPS proxy by vyžadovala samostatnou úpravu nasazení.

## Kontrola a první spuštění

Následující příkazy se provádějí až na připraveném VPS. Z rozbaleného vydání nejprve zkontrolujte konfiguraci:

```bash
sudo ./deploy/brunch.sh check
```

Kontrola používá `brunch-server --check-config`, nespouští prodej ani neodesílá e-maily. Úspěch potvrzuje místní nastavení a čitelnost klíčů, nikoli existenci SMTP účtu, dostupnost veřejného DNS nebo schválení provozovny Teya.

Při první instalaci nastavte `/opt/brunch/current` jako symbolický odkaz na zkontrolovaný adresář vydání. Pak spusťte služby a proveďte kontrolu:

```bash
sudo /opt/brunch/current/deploy/brunch.sh up
sudo /opt/brunch/current/deploy/brunch.sh health
```

Příkazy `up` a `restart` navíc porovnají všechny veřejně dostupné statické soubory s kontrolními součty konkrétního obrazu. Samostatně lze tuto kontrolu spustit příkazem `brunch.sh verify-release`. Pravidelná kontrola `health` používá lehčí kontrolu dostupnosti a místních souborů obrazu.

Při startu se aplikují databázové migrace, založí nastavení obchodu a správce. Staging ani produkce automaticky nevytvářejí ukázkové produkty. Prázdný seznam rozvozových PSČ nepovolí nové objednávky. Správce doplní skutečnou nabídku a oblasti rozvozu přes `/admin/`.

Výchozí konfiguraci lze přesměrovat přes `BRUNCH_ENV_FILE`. Například pro samostatnou staging instalaci:

```bash
sudo env BRUNCH_ENV_FILE=/etc/brunch-staging/brunch.env ./deploy/brunch.sh check
```

Před zpřístupněním zákazníkům ověřte otevření obchodu a administrace, HTTPS, přihlášení skutečným e-mailem, objednávku, změny stavu a doručení obou potvrzení. Pro Teya nejprve ověřte skutečnou staging platbu, zamítnutí, webhook a restart během platby. Výsledky simulovaných testů nepovažujte za ověření skutečného účtu.

## Zálohování a zkouška obnovy

Ruční úplnou databázovou zálohu vytvoří:

```bash
sudo /opt/brunch/current/deploy/brunch.sh backup
```

Záloha zahrnuje i fotografie, platby a e-mailovou frontu. Není to archiv konfiguračních souborů nebo tajných klíčů. Konfiguraci a klíče uchovejte samostatně v chráněném úložišti. Záložní soubor je databázový dump bez šifrování, ochranu proti čtení zajišťují oprávnění a případné šifrování cílového úložiště.

Zkoušku obnovy vybrané zálohy spouštějte explicitně:

```bash
sudo /opt/brunch/current/deploy/brunch.sh restore-check /var/backups/brunch/NAZEV_ZALOHY.dump
```

Tento příkaz obnovuje do dočasné oddělené databáze a nemění databázi obchodu. Neodesílá obnovené e-maily ani neaktivuje platební pracovníky. Úspěšná zkouška ověří čitelnost a obnovitelnost dumpu, nikoli celý postup náhrady ztraceného VPS.

Zálohovací skript používá proměnné prostředí `BRUNCH_BACKUP_DIR` a `BRUNCH_BACKUP_DAYS`, výchozí hodnoty jsou `/var/backups/brunch` a 7 dní. Tyto proměnné se předávají wrapperu nebo nastavují v systemd službě, ne v aplikačním `brunch.env`. Staré zálohy daného projektu se odstraňují až po dokončení a základní kontrole nové. Záloha má také doprovodný soubor `.sha256`, který zkouška obnovy vyžaduje.

Samotná záloha na stejném VPS nechrání před ztrátou celého serveru. Před ostrým provozem nastavte kopírování do samostatného úložiště a pravidelně ověřujte jeho výsledek. Cíl ani přístup se nyní nevymýšlejí. Uchování záloh přizpůsobte dostupnému disku a potřebám provozu. Hodinové plné zálohy s týdenním uchováním mohou vyžadovat mnohonásobek velikosti databáze, proto před zapnutím časovače ověřte volné místo.

Obnova produkční databáze vyžaduje zastavit aplikaci a řídit obnovu jako samostatný zásah. Cílovou databázi i objekty musí vlastnit aplikační role `brunch`. Při obnově pod databázovým správcem použijte `pg_restore` s `--no-owner --role=brunch`, jinak může aplikace po obnově ztratit přístup nebo oprávnění k dalším migracím. Zkouška obnovy záměrně používá jen dočasnou databázi a nenasazuje obnovená data do aplikace.

U plateb pořízených po okamžiku zálohy nejprve dohledat stav v Teya. Starší dump může postrádat přijaté peníze, novější objednávky a odeslané zprávy. Obnova nesmí automaticky vést k opakovanému účtování nebo vyřízení objednávky. Před opětovným spuštěním pracovníků porovnejte dotčené platby a objednávky.

## Automatické úlohy a dohled

Ukázkové systemd jednotky v `deploy/` používají stabilní cestu `/opt/brunch/current/deploy/brunch.sh`. `brunch-backup.timer` spouští plnou zálohu každou hodinu a `brunch-health.timer` kontrolu každých pět minut. Před instalací zkontrolujte časování, cestu konfigurace, uchování a umístění záloh. Aktivace probíhá až při nasazení:

```bash
sudo install -m 0644 deploy/brunch-*.service deploy/brunch-*.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now brunch-backup.timer brunch-health.timer
```

Výsledky kontrolujte příkazy `systemctl list-timers`, `systemctl status brunch-backup.service brunch-health.service` a v systémovém žurnálu. Neúspěch úlohy sám neposílá upozornění člověku. Příjemce a kanál upozornění je nutné připojit k provoznímu dohledu podle dodaných údajů.

Sledujte dostupnost přes veřejné HTTPS, stav kontejnerů, volný disk a paměť, úspěšné zálohy, selhané e-maily a platby ve stavu `review`. Endpoint `/api/health` kontroluje i databázi. Dostupný endpoint ale nepotvrzuje doručení e-mailů nebo funkční účet Teya.

Caddy nemá zapnuté výpisy jednotlivých HTTP požadavků. Přihlašovací URL obsahují jednorázový token, proto při budoucím zapnutí přístupových logů nejprve odstraňte citlivé parametry ze zaznamenávaných adres.

## Aktualizace a návrat k předchozí verzi

1. Připravte nové čisté vydání mimo VPS a ověřte jeho součty po přenosu.
2. Nahrajte nový obraz pomocí `docker image load`. Předchozí obraz a adresář ponechte.
3. Dosavadní instalací vytvořte čerstvou zálohu a ověřte její dokončení.
4. Přečtěte změny konfigurace a databázových migrací. Skutečný `brunch.env` zachovejte, upravte jen potřebné hodnoty včetně `BRUNCH_IMAGE`.
5. Spusťte `check` z adresáře nového vydání. Teprve po úspěchu přepněte `/opt/brunch/current` a spusťte `up` a `health`.
6. Ověřte obchod, administraci, přihlášení, statické soubory a relevantní objednávkový scénář. Zkontrolujte e-maily a rozpracované platby.

Změna tagu zpět nevrací databázové migrace. Pokud nová verze provedla nekompatibilní změnu schématu, stará aplikace nemusí umět databázi používat. Postup návratu je potřeba určit podle konkrétní migrace, často je bezpečnější opravné vydání. Obnovení předchozí zálohy navíc zahodí novější data a vyžaduje kontrolu plateb uvedenou výše.

Nikdy nepoužívejte `docker compose down -v` pro běžný restart nebo aktualizaci. Přepínač `-v` maže trvalé svazky databáze i certifikátů. Neměňte název produkčního projektu jen kvůli novému vydání, jinak se vytvoří jiná prázdná instalace.

## Co je nutné dokončit podle skutečných údajů

Před prodejem zbývá doplnit konfiguraci, doménu, DNS, externí SMTP a případně Teya. Dále je třeba zadat skutečné produkty, PSČ, cenu dopravy, kontakty a obchodní údaje. Texty podmínek a informací o zpracování osobních údajů musí odpovídat skutečnému provozovateli a způsobu prodeje. Příprava infrastruktury tyto údaje nedoplňuje.

Ověření skutečného obrazu, HTTPS certifikátu, externího doručení, offsite zálohy a Teya se zapisuje až po jejich provedení. Tento návod ani úspěšná místní kontrola konfigurace takový výsledek nenahrazují.
