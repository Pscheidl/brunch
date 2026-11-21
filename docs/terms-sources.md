# Podklady k obchodním podmínkám

Výchozí text k 21. 11. 2026 je v `server/src/commerce/initial-terms-cs.txt`. Provozovatel výslovně zadal název Mysterium Gaudi, dočasné IČO 11111111 a pozdější doplnění sídla i veřejných kontaktů. První oddíl tyto hodnoty transparentně označuje jako neúplné a dočasné. Nejde o ověřené úplné identifikační údaje. Soubor je výchozí text pro první verzi, následné úpravy vznikají jako nové databázové verze v administraci bez restartu. Zveřejnění tohoto prozatímního znění vychází z výslovného zadání provozovatele.

## Zdroje

- [ČOI: spotřebitelský průvodce](https://coi.gov.cz/pro-spotrebitele/spotrebitelsky-pruvodce/) pro předsmluvní informace a zákonná práva spotřebitele.
- [ČOI: potvrzení objednávky](https://coi.gov.cz/faq/2-musi-mi-podnikatel-potvrdit-objednavku-2/) pro doručení smluvních údajů a přijatého znění podmínek e-mailem.
- [ČOI: odstoupení u zboží](https://coi.gov.cz/faq/7-odstoupeni-od-smlouvy-do-14-dnu-u-zbozi/) pro čtrnáctidenní lhůtu, konkrétní výjimku u rychle se kazícího zboží a postup vrácení.
- [ČOI: reklamace zboží a služeb](https://coi.gov.cz/pro-podnikatele/informace-pro-prodejce-zbozi-a-sluzeb/reklamace-zbozi-a-sluzeb/) pro místa uplatnění, potvrzení a třicetidenní vyřízení reklamace.
- [ČOI: odstoupení pro vady](https://coi.gov.cz/faq/2-kdy-mohu-pro-vady-zbozi-odstoupit-2/) pro zákonnou posloupnost nároků z vad.
- [ČOI: mimosoudní řešení sporů](https://coi.gov.cz/informace-o-adr/) pro aktuální cestu k ADR.
- [MPO: ukončení platformy ODR](https://mpo.gov.cz/cz/ochrana-spotrebitele/mimosoudni-reseni-spotrebitelskych-sporu-adr/ukonceni-cinnosti-evropske-platformy-pro-reseni-sporu-on-line-se-blizi--286849/) vysvětluje konec platformy a informační povinnosti k 20. 7. 2025. Návrh proto neobsahuje zastaralý odkaz na ODR.
- [ÚOOÚ: informace pro e-shopy](https://uoou.gov.cz/profesional/qa-otazky-a-odpovedi/informace-pro-e-shopy) pro rozlišení smlouvy, zákonné povinnosti a oprávněného zájmu od souhlasu.
- [ÚOOÚ: kontakt](https://uoou.gov.cz/kontakt) pro adresu dozorového úřadu.

## Provozní předpoklady k ověření

Návrh vychází z aktuálního objednávkového tlačítka, potvrzovacích e-mailů, stavů objednávek a kalendáře v aplikaci. Uzavření smlouvy váže na e-mail o přijetí objednávky ke zpracování, u online platby po ověření úhrady. Následné ruční potvrzení upřesňuje termín rozvozu. Jde o zvolený smluvní model, který musí provozovatel přijmout a dodržovat. Pokud má smlouva vznikat až ručním přijetím, je třeba současně upravit jednoznačně i potvrzovací e-maily a text pokladny.

Cena dopravy, hranice dopravy zdarma a časová okna se přebírají z pokladny bez pevně vymyšlených částek. Rozvoz respektuje soboty, neděle i české svátky v `src/working_day.rs`. Návrh nepředpokládá paušální právo stornovat již přijatou objednávku ani náhradu výrobku bez souhlasu zákazníka.

Provozovatel musí doplnit skutečné identifikační a kontaktní údaje, potvrdit adresu pro reklamace a vrácení a zajistit správné složení, alergeny, skladovací pokyny a označení doby spotřeby u konkrétních potravin. Výjimka z odstoupení se neposuzuje plošně pro celý katalog.

Oprávněný zájem pro evidenci IP je navržený účel, nikoli právní závěr o libovolném rozsahu uchování. Provozovatel má ověřit nezbytnost a přiměřenost, skutečné poskytovatele služeb a zavést výmaz nebo anonymizaci podle doložených retenčních pravidel. Návrh neslibuje konkrétní počet let bez znalosti účetních a provozních požadavků. Souhlas s podmínkami nezakládá marketingový souhlas.

Jde o návrh pro kontrolu provozovatelem podle skutečného provozu, nikoli o záruku úplného právního posouzení konkrétního podnikání.
