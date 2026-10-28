# Soukromý náhled Brunch Era přes SSH

Tento režim umožňuje ověřit instalaci před dodáním domény, skutečné pošty a platební brány. Není veřejným obchodem. Web poslouchá na VPS pouze na `127.0.0.1:3100`, administrace je dostupná přes SSH tunel a všechny zprávy zachytává interní Mailpit. Kdo má přístup k tomuto tunelu a Mailpitu, může získat přihlašovací odkaz správce. SSH přístup proto patří pouze správcům.

`APP_ENV=preview` vyžaduje HTTP adresu na loopbacku, povoluje pouze místní SMTP bez přihlašovacích údajů a zakazuje Teya. Nevytváří ukázkové zboží. Produkční a stagingové kontroly nadále vyžadují skutečnou doménu HTTPS a SMTP s TLS. Náhled má vlastní databázový svazek v projektu `brunch-preview`.

## Příprava a spuštění

Nejprve zabezpečit VPS a přenést ověřené vydání podle [postupu pro Zoner Cloud](zoner-cloud.md). Rust a frontend sestavit mimo VPS. Aktuální vydání má být dostupné přes `/opt/brunch/current`. Na serveru vytvořit `/etc/brunch/preview.env` z `deploy/.env.preview.example`, s vlastníkem root a právy `0600`. Vyplnit tag přeneseného obrazu a dvě různá náhodná databázová hesla, každé jako 64 hexadecimálních znaků. Soubor není shellový skript, nepoužívat `source`.

Příkazy v dalších blocích patří do bash relace na VPS:

```bash
dc() {
    sudo docker compose -p brunch-preview \
        --env-file /etc/brunch/preview.env \
        -f /opt/brunch/current/deploy/compose.preview.yaml "$@"
}
dc config -q
dc run --rm --no-deps app --check-config
dc up -d --wait --wait-timeout 180
dc ps
```

Tento Compose je samostatný. Nekombinovat jej s `compose.yaml` a nepoužívat produkční `deploy/brunch.sh`, `brunch-health.timer` ani `brunch-backup.timer`. Produkční wrapper záměrně vyžaduje veřejné HTTPS a produkční konfiguraci. Po restartu VPS kontejnery náhledu obnoví Docker.

## Web a pošta přes tunel

Na počítači správce otevřít tunel. `BRUNCH_SSH_ALIAS` nahradit připraveným SSH aliasem tohoto VPS:

```bash
ssh -N -o ExitOnForwardFailure=yes \
    -L 127.0.0.1:3100:127.0.0.1:3100 BRUNCH_SSH_ALIAS
```

Web je pak na `http://127.0.0.1:3100`, administrace na `http://127.0.0.1:3100/admin/`. Používat stejný místní port a adresu, protože přihlašovací odkazy a kontrola původu požadavků vycházejí z této URL. Přenos mezi počítačem a VPS chrání SSH.

Mailpit nemá publikovaný SMTP ani HTTP port a nemá síťovou cestu ven. Na VPS zjistit jeho aktuální interní IP:

```bash
mailpit_container=$(dc ps -q mailpit)
sudo docker inspect --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$mailpit_container"
```

Na počítači správce otevřít druhý tunel. `MAILPIT_IP` nahradit právě zjištěnou adresou:

```bash
ssh -N -o ExitOnForwardFailure=yes \
    -L 127.0.0.1:8025:MAILPIT_IP:8025 BRUNCH_SSH_ALIAS
```

Mailpit otevřít na `http://127.0.0.1:8025`. V administraci obchodu požádat o odkaz pro `admin@example.test` a otevřít jej ze zachycené zprávy. Na žádnou skutečnou schránku se zpráva neodesílá. Po znovuvytvoření kontejneru zjistit jeho IP znovu. Tunely ukončit pomocí Ctrl+C.

Použitý obraz `axllent/mailpit:v1.31.3` používá stejnou verzi jako konfigurace obecního webu. Zdroj: [oficiální vydání Mailpit](https://github.com/axllent/mailpit/releases/tag/v1.31.3) a [oficiální dokumentace obrazů](https://mailpit.axllent.org/docs/install/docker/). Mailpit uchovává nejvýše 100 zpráv a odstraňuje zprávy starší než sedm dní. Zachycené přihlašovací odkazy jsou citlivé, Mailpit nikdy nepublikovat na veřejné adrese.

## Kontroly a ruční záloha

Na VPS ověřit dostupnost, integritu souborů a prázdný počáteční katalog:

```bash
curl --fail --silent --show-error http://127.0.0.1:3100/api/health
dc exec -T app sh -c 'cd /app && sha256sum -c assets.sha256'
dc exec -T db psql -U postgres -d brunch -Atc 'SELECT count(*) FROM products'
dc logs --tail 50 app
```

Po prvním spuštění má katalog nula produktů. Data pro ověření zadává správce. Ověřit přihlášení, změnu nastavení a zachycení zpráv. Z jiného stroje ověřit, že porty `3100`, `5432`, `1025` a `8025` nejsou veřejně dostupné, a kontrolu zopakovat po restartu. Port 80 ani 443 tento režim nepotřebuje.

Automatické zálohy nejsou tímto Compose zapnuté. Před aktualizací lze pořídit soukromý dump na VPS. Tento blok používá funkci `dc` výše a při selhání ponechá pouze soubor `.partial`:

```bash
set -o pipefail
sudo install -d -m 0700 /var/backups/brunch-preview
preview_dump="/var/backups/brunch-preview/brunch-$(date -u +%Y%m%dT%H%M%SZ).dump"
dc exec -T db pg_dump -U postgres -d brunch --format=custom --no-owner --no-privileges |
    sudo sh -c 'umask 077; cat > "$1"' sh "$preview_dump.partial" &&
sudo sh -c 'docker compose -p brunch-preview --env-file /etc/brunch/preview.env -f /opt/brunch/current/deploy/compose.preview.yaml exec -T db pg_restore --list < "$1" >/dev/null' sh "$preview_dump.partial" &&
sudo mv "$preview_dump.partial" "$preview_dump"
```

Dump obsahuje údaje z náhledu. Před jeho přenosem určit soukromé cílové úložiště a ověřit obnovitelnost. Záloha na stejném disku není ochrana proti ztrátě VPS. Nespouštět `down -v`, tento přepínač by smazal databázi i zachycené zprávy.

## Přechod na veřejný obchod

Po dodání domény, DNS, skutečného správce a SMTP připravit oddělený soubor `/etc/brunch/brunch.env`, projekt `brunch-production` a jeho vlastní databázi podle [produkčního postupu](zoner-cloud.md). Náhledové adresy, zprávy a zkušební objednávky nepřenášet do ostrého provozu. Novou produkci ověřit přes HTTPS včetně skutečného doručení přihlašovacího e-mailu. Teya zapnout až po dodání a ověření správného produkčního účtu. Náhled lze pak zastavit příkazem `dc stop`, data zůstanou zachovaná.

Správce náhledu se automaticky nevytváří. První účet založte jednorázovým `brunch-server --initialize-admin EMAIL` v aplikačním kontejneru, s heslem předaným standardním vstupem. Další účty spravuje sekce Administrátoři. Zákaznické odkazy zachycené v Mailpitu neudělují administrátorský přístup.
