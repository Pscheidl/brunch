# Osobní odběr

Osobní odběr je samostatná volba vedle rozvozu. Je bez poplatku a nevyžaduje doručovací adresu zákazníka ani PSČ z oblasti rozvozu. Zákazník zadá kontaktní údaje, vybere pracovní den a časové okno. Nejbližší možný termín je následující pracovní den, české svátky se nenabízejí. Termín potvrdí obsluha.

Pro spuštění je určeno odběrné místo Mysterium Gaudi na Strahově, parkování zdarma a odběr v pracovní dny v okně **08:00–17:00**. Tyto provozní hodnoty se zadávají v administraci. Migrace sama žádnou konkrétní adresu ani čas nezakládá a ponechá odběr vypnutý, dokud není nastaven.

## Nastavení a změna adresy

V administraci otevřete **Doprava a nastavení → Osobní odběr**. Vyplňte název místa, ulici a číslo, město, PSČ, časová okna a případné pokyny ke vstupu. Volba **Parkování zdarma** je zapnutá ve výchozím formuláři. Poté zapněte **Povolit osobní odběr** a použijte **Uložit osobní odběr**.

- Zapnutí vyžaduje celý název a adresu, pětimístné PSČ a alespoň jedno platné časové okno.
- Časová okna se zadávají po jednom na řádek, například `08:00–17:00`. Mohou být nejvýše 24, nesmějí se překrývat a konec musí být po začátku.
- Adresu, okna, pokyny i údaj o parkování lze změnit stejným formulářem. Neúplné nastavení lze uložit jako vypnuté.
- Tlačítko **Obnovit osobní odběr** načte uložené údaje. Před zahozením rozepsané změny se zobrazí potvrzení.
- Pokud nastavení mezitím upravil jiný správce, uložení skončí konfliktem. Obnovte aktuální údaje a zkontrolujte vlastní změny.

Uložení odběru nemění cenu rozvozu, rozvozová PSČ ani příjemce objednávek. Vypnutí zabrání novým objednávkám s osobním odběrem, již přijaté objednávky zůstávají zachované.

## Vyřízení objednávky

V seznamu a detailu objednávky je uvedeno **Osobní odběr**. Místo, pokyny a parkování vidí také zákazník v potvrzení a svém účtu. Informace v objednávce zachycují stav při jejím přijetí. Pozdější změna adresy v nastavení nepřepíše staré objednávky ani vystavené faktury. Pokud je potřeba přesunout již přijatý odběr, vyřiďte změnu se zákazníkem samostatně.

Postup obsluhy je **Přijatá → Potvrzená → Připraveno k vyzvednutí → Vyzvednuto**. Označení připravené objednávky odešle zákazníkovi zprávu. Při převzetí potvrďte přijetí hotovosti nebo platby kartou a následně stav Vyzvednuto. U ověřené online platby již peníze nevybírejte. Hromadná změna v administraci vybírá objednávky stejného způsobu převzetí a stejného stavu.

Osobní odběr má vždy poplatek **0 Kč**, který kontroluje server. Ceny zboží, minima a sklad se ověřují stejně jako u rozvozu. Změní-li se odběrné místo nebo časy během rozpracované nové objednávky, zákazník musí zkontrolovat aktuální údaje. Přesné opakování již přijatého požadavku vrátí původní objednávku.

## Fakturační údaje

Adresa prodejny není fakturační adresou zákazníka. U osobního odběru se použijí výslovně zadané fakturační údaje. Pokud chybějí, formulář zákazníka i správce ponechá adresu odběratele prázdnou. Vystavená faktura uchová údaje zákazníka i původní odběrné místo, včetně informace o parkování.

## Ověření a nasazení

Nasazení vyžaduje migraci `0025_pickup.sql`, nový backend i sestavený obchod, administraci a zákaznickou stránku. Před migrací použijte standardní zálohovací postup projektu. Nastavení je v `GET/PUT /api/admin/pickup-settings`, veřejná nabídka ho dostává v poli `pickup` z `/api/settings`.

Testy používají izolované místní prostředí:

- `server/tests/pickup.rs` ověřuje nastavení, oprávnění, nulový poplatek, platné termíny, snapshot místa a opakování požadavků.
- `server/tests/customer_experience.rs` ověřuje také oddělenou fakturační adresu a zachování původního místa na faktuře při změně nebo vypnutí odběru.
- `scripts/test-pickup-flow-browser.cjs` provede skutečný nákup přes místní backend a PostgreSQL, ověří zachycený potvrzovací e-mail a neměnnou adresu po změně nastavení. Vyžaduje `TEST_ADMIN_PASSWORD` testovacího správce a místní Mailpit.
- `scripts/test-pickup-storefront-browser.cjs` ověřuje košík, přepínání převzetí a obnovování rozpracované objednávky.
- `scripts/test-pickup-admin-browser.cjs` ověřuje nastavení, validace, konflikty a obsluhu objednávek.
- `scripts/test-pickup-invoice-browser.cjs` a `scripts/test-customer-browser.cjs` ověřují fakturační formuláře, účet, mobilní rozměry a tisk.

Při spuštění databázových testů musí `TEST_DATABASE_URL` ukazovat na izolovanou testovací PostgreSQL. Prohlížečové testy vyžadují místní server a Playwright. Produkční objednávky ani skutečné e-maily se tímto testovacím postupem nevytvářejí.
