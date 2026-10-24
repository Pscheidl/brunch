# Online platby Teya

Integrace používá Teya Hosted Checkout. Údaje karty zadává zákazník přímo na stránce Teya. Brunch nepřijímá ani neukládá číslo karty, CVC nebo přístupový token do prohlížeče. Cena vzniká na serveru ze skutečného katalogu, částky jsou celé haléře v CZK.

## Stavy objednávek

- `card_on_delivery` a `cash_on_delivery`: ihned `new`, v rozhraní „Přijata“, platba `unpaid` až do převzetí.
- `card`: `awaiting_payment`, v rozhraní „Čeká na zaplacení“. Sklad je rezervovaný, obchod zatím nedostává objednávku k vyřízení.
- Po ověření skutečné online úhrady před koncem lhůty: `new` a `paid`, příslušný záznam v `payment_receipts`, historie a oba potvrzovací e-maily vzniknou jednou databázovou transakcí.

Správce nemůže online objednávku ručně označit za zaplacenou ani posunout do přípravy bez ověřené úhrady. Uhrazenou objednávku nelze stornovat, automatické refundace nejsou implementované. Nezaplacenou online objednávku lze ručně stornovat až po ověření konečného `EXPIRED` nebo `COMPLETED/FAILED` u Teya. Sklad se při stornu vrátí jednou. Pouhé zavření prohlížeče ani návrat přes cancel URL takové ověření nenahrazují. Samostatná automatická expirace objednávky se řídí uloženou lhůtou níže.

## Automatická kontrola a expirace objednávek

Server sám kontroluje známé nedokončené platby přibližně každých 30 sekund, zákazník nemusí mít otevřenou stránku. Po chybě API se interval prodlužuje, úloha pokračuje po restartu. Odmítnutí a konec relace se ověřují přes API, protože Teya posílá webhook pouze k úspěchu.

Výchozí doba platnosti nezaplacené online objednávky je 60 minut. V prostředí serveru ji lze nastavit:

```dotenv
ONLINE_PAYMENT_TIMEOUT_MINUTES=60
```

Povoleny jsou celé minuty od 1 do 1440. Hodnota se počítá od vytvoření objednávky a výsledný `expires_at` se uloží do PostgreSQL ve stejné transakci. Změna konfigurace platí až pro nové objednávky. Restart ani opakované odeslání již uloženou lhůtu neprodlužují. Migrace dřívějším online objednávkám nastaví termín jednu hodinu od jejich původního vytvoření.

Samostatná údržba kontroluje termíny každou sekundu a nepotřebuje spojení s Teya. Po termínu převede dosud neověřenou online objednávku do konečného stavu `expired`, v rozhraní „Vypršela“. Vrácení skladu, historie a zpráva zákazníkovi jsou jedna transakce, opakování ani souběh nevrátí sklad dvakrát. Při vypnutém serveru nebo nedostupné databázi se údržba provede po obnovení provozu podle původního termínu. Platby při doručení a již ověřené uhrazené objednávky neexpirují.

Po termínu se zákazníkovi nevrací odkaz na platbu ani se nezakládá nová relace. Lhůta se znovu kontroluje také uvnitř transakce potvrzující platbu, takže ji nelze obejít tím, že údržba ještě neběžela. Známou relaci server může dál ověřovat kvůli pozdnímu potvrzení. Pokud se úhrada prokáže až po termínu, uchová se doklad o přijatých penězích, proces přejde do `review` s důvodem `paid_expired_order` a obsluha i zákazník dostanou upozornění. Objednávka se znovu nepřijme a uvolněný sklad se znovu nerezervuje. Refundaci nebo jinou dohodu se zákazníkem řeší obsluha.

**Lhůta objednávky je odlišná od platební relace.** Teya Hosted Checkout podle dokumentace sám expiruje po 15 minutách a publikované rozhraní nemá parametr pro změnu této doby ani příkaz k předčasnému zrušení relace. Lokální hodinová lhůta jeho relaci neprodlužuje. Již otevřenou stránku brány také nelze lokální expirací přímo zneplatnit, proto se pozdní úhrada nikdy nezahazuje. Při nejasném výsledku se automaticky nezakládá náhradní platba. [Dokumentace Teya k expiraci relace](https://docs.teya.com/online-payments/hosted-checkout/customize#session-expiry).

## Nastavení přístupových údajů

Bez `TEYA_ENVIRONMENT` zůstává online volba vypnutá a server odmítne nové online objednávky. Stávající objednávky zůstávají v databázi. Pro zpracování rozpracovaných plateb musí zůstat příslušná konfigurace a veřejný webhook aktivní.

1. U Teya získat přístup k developerskému prostředí a přihlašovací údaje konkrétní provozovny. Použít pouze oprávnění `checkout/sessions/create` a `checkout/sessions/id/get`. Refundace aplikace nevyžaduje.
2. `client_secret` uložit v souboru mimo repozitář, zálohy zdrojů a adresáře `public`/`dist`. Vhodný je provozní správce tajných údajů nebo soubor vlastněný účtem služby s oprávněními `0600` či `0400`. Aplikace jiná oprávnění odmítne. Soubor nesmí být dostupný webovým serverem.
3. Nastavit následující proměnné v prostředí služby. Nejde o údaje pro zákaznickou ani běžnou administrační stránku:

```dotenv
PUBLIC_URL=https://vas-obchod.example
ONLINE_PAYMENT_TIMEOUT_MINUTES=60
TEYA_ENVIRONMENT=staging
TEYA_STORE_ID=identifikator-provozovny
TEYA_CLIENT_ID=identifikator-klienta
TEYA_CLIENT_SECRET_FILE=/etc/brunch/secrets/teya-client-secret
TEYA_WEBHOOK_PUBLIC_KEY_FILE=/etc/brunch/teya-webhook-public.pem
TEYA_WEBHOOK_PREVIOUS_PUBLIC_KEY_FILE=
```

Veřejný klíč pro ověřování webhooků získat z Teya portálu důvěryhodnou cestou. Očekává se RSA veřejný PEM klíč `BEGIN PUBLIC KEY`, alespoň 2048 bitů. Není to privátní klíč obchodníka. Při rotaci lze po přechodnou dobu nakonfigurovat předchozí veřejný klíč ve druhém souboru. Klíče ani tajné údaje se nestahují z URL dodané ve webhooku. Konfigurace se načítá při startu služby, po výměně klíče je potřeba restart.

4. V Teya portálu nastavit událost `payment.succeeded.v1` na `https://vas-obchod.example/api/payments/teya/webhook`. Endpoint musí být veřejně dostupný přes HTTPS, bez přihlášení, bez HTML výzvy ochrany proti robotům a bez přepisování těla požadavku. Ostatní zápisové API stále vyžaduje ochranu požadavku proti cizímu původu.
5. Ověřit ve stagingu skutečnou platbu, zamítnutí, návrat zákazníka, doručení webhooku a restart aplikace během platby. Teprve potom přepnout samostatnou produkční instalaci na `production` s produkčními údaji a webhookovým klíčem. Staging a produkce nesmějí sdílet objednávkovou databázi.

Adresy OAuth a API jsou pevně určené prostředím: `id.teya.xyz`, `api.teya.xyz` pro staging, `id.teya.com`, `api.teya.com` pro produkci. Server neposílá tajné údaje na nastavitelnou libovolnou URL a nesleduje HTTP přesměrování API. Přesměrování zákazníka povoluje pouze HTTPS na `checkout.teya.com`, ve stagingu také `checkout.teya.xyz`. Dokumentace uvádí příklady s `.com`, skutečnou URL daného staging účtu je ještě potřeba ověřit. Žádný tajný údaj není součástí veřejného API ani uloženého těla požadavku na platbu.

## Restart, výpadky a opakovaná oznámení

| Okamžik přerušení | Obnova |
| --- | --- |
| Před potvrzením databázové transakce objednávky | Objednávka, sklad i platební úloha se vrátí společně. Opakování se stejným klíčem je bezpečné. |
| Po uložení objednávky, před voláním Teya | Úloha i termín jsou v PostgreSQL. Pracovník ji načte po startu, po termínu už novou platební relaci nevytvoří. |
| Teya založila relaci, odpověď se ztratila | Opakuje se stejné tělo a stejné uložené `Idempotency-Key`, nikdy nový klíč. |
| Po otevření platební stránky | Relace Teya a její vazba na objednávku už jsou v databázi. Platba pokračuje u Teya i bez běžícího Brunch serveru. |
| Webhook přijde během výpadku | Teya oznámení opakuje. Po startu současně běží vlastní dotazování známých relací, takže samotné vyčerpání opakování webhooku neztratí známou platbu. |
| Po přijetí webhooku, před ověřením platby | Podpisem ověřené oznámení i úloha jsou trvale uložené před odpovědí 204. Po startu se stav ověří v API. |
| Během přijetí platby do objednávky | Úhrada, stav, historie a e-mailová fronta se potvrdí společně nebo se společně vrátí. |
| Po potvrzení v databázi, před odpovědí | Další oznámení ani kontrola nevytvoří druhou úhradu nebo druhé potvrzení objednávky. |

Úlohy mají dvouminutový zámek s identifikátorem pracovníka. Opuštěný zámek po pádu vyprší. Starý pracovník nesmí přepsat výsledek nového. Aktivní relace se kontrolují přibližně po 30 sekundách. Při selhání API se odstup zvyšuje od 15 sekund až k hodině, pro známé relace bez konečného stavu se pokračuje i po restartu.

Teya dokumentuje celkem šest pokusů o webhook, poslední přibližně 9 hodin a 20 minut po prvním. Nenahrazuje to vlastní trvalou frontu ani dohled. Podpis se ověřuje přes přesné původní bajty těla pomocí RSA PKCS#1 v1.5 a SHA-256, hlavička `x-teya-signature` je Base64. Podpis sám ještě neznamená přijetí objednávky. Pracovník vždy ověří přes OAuth API relaci, provozovnu, referenci objednávky, částku, měnu, typ `SALE` a kombinaci `COMPLETED/SUCCESS`. Samotná autorizace, cizí provozovna nebo úspěšná návratová URL nestačí.

Identifikátor neznámé relace z oznámení se ukládá pouze jako kandidát. Dokud server neověří provozovnu a další údaje u Teya, nesmí nahradit skutečnou relaci objednávky. To chrání obnovu i pro případ, že Teya používá podpisový klíč společný více obchodníkům.

Teya v ověřené dokumentaci neuvádí dobu uchování idempotentních požadavků. Proto požadavek bez známého ID relace po konci lhůty objednávky nebo starší než hodinu nepouštíme automaticky znovu, přejde do `review`. Je to konzervativní místní hranice, nikoli záruka Teya o době uchování klíče. Pozdější správně podepsané oznámení umí dodat kandidáta relace a obnovit ověření. Pro tuto výjimečnou situaci musí obsluha nejprve dohledat stav v Teya, nesmí pouze vytvořit novou platbu nebo označit objednávku za uhrazenou.

## Provozní dohled

Detail online objednávky v administraci ukazuje stav platebního procesu, čas poslední kontroly a případný důvod potřeby kontroly. Po ověření úhrady obsahuje také oddíl „Přijatá online platba“ s částkou, měnou, identifikátorem transakce Teya, identifikátorem relace a časem ověření. Údaje se čtou z trvalé tabulky `payment_receipts`, zachovají se po restartu a jsou dostupné pouze správci. Záznam vzniká i pro pozdní platbu uzavřené objednávky, spolu s upozorněním k ručnímu vyřešení.

Sledujte dlouho čekající objednávky, stav `review`, chyby pracovníka a dostupnost databáze. `configuration_changed` znamená, že uložená provozovna nebo prostředí neodpovídá konfiguraci. `provider_data_mismatch` znamená nesoulad autoritativních údajů. `receipt_conflict` signalizuje konflikt jedinečnosti úhrady. `create_outcome_unknown` vyžaduje dohledání relace. `order_deadline_passed` znamená, že po lhůtě už nebyla založena nová relace. `paid_expired_order` a `paid_cancelled_order` označují přijaté peníze k již uzavřené objednávce. Tyto stavy se nesmějí obcházet ruční změnou `payment_status`.

Zrušená nebo expirovaná objednávka, u níž později API prokáže úhradu, má doklad o přijatých penězích uložený a přejde platebním procesem do `review`. Objednávka se automaticky znovu neotevře. Obsluha i zákazník dostanou upozornění právě jednou. Obsluha musí vyřešit peníze, případně refundaci přes Teya. Vlastní refundace a automatická synchronizace pozdějších refundací či chargebacků nejsou součástí této integrace.

Zálohovat PostgreSQL včetně platebních tabulek a e-mailové fronty. Nemažte platební tabulky při restartu ani neobnovujte samotné objednávky bez plateb. Příjem peněz nelze bezpečně obnovit z paměti aplikace nebo ze session prohlížeče.

Zákaznický prohlížeč uchovává v session storage původní klíč a uloženou objednávku. Po návratu pouze dotazuje server. Ztráta této session zabrání obnovení přístupu hosta v daném prohlížeči, nezruší uloženou platbu, potvrzení e-mailem ani serverové ověřování. Stavová cesta neposkytuje osobní údaje a vyžaduje původní náhodný klíč v JSON těle, nikdy v URL.

## Ověření a zdroje

Místní testy používají skutečný PostgreSQL, HTTP server a simulátor dokumentovaného Teya API s RSA podpisy. Prohlížečové testy spouštějí skutečnou aplikaci s řízenými odpověďmi API. Skutečný účet Teya, staging platba a produkční HTTPS webhook zatím nebyly dostupné, tyto kroky jsou nezbytné před zapnutím plateb zákazníkům.

- [Teya: vlastní integrace v ČR](https://help.teya.com/hc/cs-cz/articles/47742512702353-Vytvo%C5%99te-si-vlastn%C3%AD-integraci-s-Teya)
- [Hosted Checkout a OAuth](https://docs.teya.com/online-payments/hosted-checkout/get-started)
- [Online Payments API](https://docs.teya.com/apis/online-payments/apis)
- [Webhooky, podpis a opakování](https://docs.teya.com/online-payments/webhooks)

Dokumentace ověřena 24. 10. 2026. Při změně API znovu ověřit kontrakt a staging scénáře.
