# brunch

Samostatný frontend pekárny s rozvozem svačin do kanceláře. Prototyp běží celý v prohlížeči a je připravený na GitHub Pages. Používá Rust 1.98.1, Leptos 0.8.21 v režimu CSR a Trunk 0.21.14. Backend, API ani databáze nejsou potřeba.

Schválený vzhled kombinuje bílý základ, olivovou, světlou šalvěj a meruňku. Písmo Geist a jemné linky navazují na projekt `obecni-web`.

## Co lze vyzkoušet

- Nabídku šesti svačin, kategorie, rostlinný filtr a detaily se složením a alergeny.
- Přidávání a množství, výpočet dopravy a košík zachovaný po obnovení stránky.
- Objednávkový tok **Košík → Doručení → Platba → Potvrzení ukázky**, včetně návratu mezi kroky.
- Ukázkovou platbu kartou nebo při převzetí. Prototyp nevyžaduje skutečné údaje karty.
- Pobočky dostupné z navigace. Konkrétní adresy a otevírací dobu zatím čekáme od pekárny.

Objednávky ani platby se nikam neodesílají. Kontaktní údaje existují pouze v paměti otevřené stránky a po obnovení zmizí. Do místního úložiště se zapisují pouze počty svačin. Produkty, ceny, doprava, termíny a fotografie jsou ukázkové.

## Místní spuštění

Jednorázově nainstalujte Rust s cílem WebAssembly a Trunk:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked --version 0.21.14
```

Spusťte vývojový náhled:

```bash
./scripts/dev.sh
```

Otevřete <http://127.0.0.1:3100>. Trunk při změnách obnovuje stránku. Jde pouze o vývojový server statických souborů.

Skripty používají Trunk z `PATH`, `target/tools/trunk/trunk` nebo místního sousedního projektu `../obecni-web/target/tools/trunk/trunk`. Vlastní cestu lze předat přes `BRUNCH_TRUNK_BIN`. Sousední projekt není pro sestavení potřeba, pokud je Trunk nainstalovaný.

## Sestavení pro GitHub Pages

```bash
./scripts/build.sh --public-url /brunch/
```

Hotové soubory jsou v `dist/`. Pro doménu bez podadresáře použijte `--public-url /`. Výstup obsahuje HTML, CSS, JavaScript, WebAssembly, písma a optimalizované fotografie. Zdrojové PNG, návrhové poznámky ani lokální screenshoty se nepublikují.

Pro offline sestavení ze stažených závislostí a nástrojů:

```bash
CARGO_NET_OFFLINE=true ./scripts/build.sh --offline --public-url /brunch/
```

Postup nasazení:

1. Připojte projekt k repozitáři na GitHubu a nahrajte zdrojové soubory na větev `main`.
2. V **Settings → Pages → Build and deployment → Source** vyberte **GitHub Actions**.
3. Workflow `.github/workflows/pages.yml` provede kontroly, sestaví statický web a zveřejní jej. Cestu webu přebírá z nastavení Pages, podporuje tedy projektový podadresář i kořen vlastní domény.
4. Veřejnou adresu najdete ve výsledku nasazení a v nastavení Pages.

Pull requesty pouze ověřují sestavení. Nasazení probíhá z `main`, automaticky po pushi nebo ručním spuštění workflow. Účty a skutečné platby přijdou až s backendem.

## Ověření

```bash
cargo fmt --check
cargo test --locked --lib
./scripts/build.sh --public-url /brunch/
python3 scripts/check-pages.py --dist dist --public-url /brunch/
```

CI kromě testů košíku ověřuje sestavení pro kořen i podadresář a kontroluje reference v HTML, CSS a JavaScriptu proti skutečným souborům v balíčku. Interakce je potřeba ověřit v prohlížeči.

## Struktura

- `src/app.rs`: komponenty a objednávkový tok
- `src/catalog.rs`: ukázkový katalog a výpočty košíku
- `src/main.rs`: spuštění Leptos aplikace v prohlížeči
- `index.html`, `Trunk.toml`: statický vstup a sestavení
- `style/`: schválený vzhled a styly objednávky
- `public/`: fotografie, písma a malý indikátor načítání
- `.github/workflows/pages.yml`: kontroly a nasazení

Podklady návrhu a jejich omezení jsou v [poznámce k UX](design/ux-decisions.md). Prompty fotografií jsou v `design/image-prompts.txt`, licence písem v `public/fonts/LICENSE.txt`.

Sestavení odpovídá oficiálním postupům [Leptos CSR](https://book.leptos.dev/deployment/csr.html) a [GitHub Pages](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages).
