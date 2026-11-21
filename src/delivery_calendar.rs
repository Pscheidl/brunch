use crate::working_day::working_day;
use chrono::{Datelike, Days, Months, NaiveDate};
use leptos::prelude::*;

const WEEKDAYS: [&str; 7] = [
    "Pondělí",
    "Úterý",
    "Středa",
    "Čtvrtek",
    "Pátek",
    "Sobota",
    "Neděle",
];
const MONTHS: [&str; 12] = [
    "Leden",
    "Únor",
    "Březen",
    "Duben",
    "Květen",
    "Červen",
    "Červenec",
    "Srpen",
    "Září",
    "Říjen",
    "Listopad",
    "Prosinec",
];

pub fn prague_today() -> Option<NaiveDate> {
    #[cfg(target_arch = "wasm32")]
    {
        let options = js_sys::Object::new();
        for (key, value) in [
            ("timeZone", "Europe/Prague"),
            ("year", "numeric"),
            ("month", "2-digit"),
            ("day", "2-digit"),
            ("calendar", "gregory"),
            ("numberingSystem", "latn"),
        ] {
            js_sys::Reflect::set(&options, &key.into(), &value.into()).ok()?;
        }
        let value: String = js_sys::Date::new_0()
            .to_locale_date_string("sv-SE", &options)
            .into();
        value.parse().ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    None
}

pub fn earliest_date(
    server_date: Option<NaiveDate>,
    today: Option<NaiveDate>,
) -> Option<NaiveDate> {
    let mut earliest = server_date?;
    if let Some(today) = today {
        earliest = earliest.max(today.checked_add_days(Days::new(1))?);
    }
    while !working_day(earliest) {
        earliest = earliest.checked_add_days(Days::new(1))?;
    }
    Some(earliest)
}

pub fn selectable_date(date: NaiveDate, earliest: Option<NaiveDate>) -> bool {
    earliest.is_some_and(|first| date >= first) && working_day(date)
}

pub fn date_label(date: NaiveDate) -> String {
    format!(
        "{} {}. {}. {}",
        WEEKDAYS[date.weekday().num_days_from_monday() as usize],
        date.day(),
        date.month(),
        date.year()
    )
}

#[component]
pub fn DeliveryCalendar(
    day: RwSignal<String>,
    earliest: Signal<Option<NaiveDate>>,
) -> impl IntoView {
    let month = RwSignal::new(None::<NaiveDate>);
    Effect::new(move |_| {
        let selected = day
            .get()
            .parse::<NaiveDate>()
            .ok()
            .and_then(|d| d.with_day(1));
        if month.get_untracked() != selected {
            month.set(selected);
        }
    });
    let shift = move |forward| {
        month.update(|month| {
            if let Some(start) = *month {
                let next = if forward {
                    start.checked_add_months(Months::new(1))
                } else {
                    start.checked_sub_months(Months::new(1))
                };
                if let Some(next) = next.filter(|next| {
                    earliest
                        .get_untracked()
                        .is_some_and(|first| *next >= first.with_day(1).unwrap())
                }) {
                    *month = Some(next);
                }
            }
        });
    };
    view! {
        <div class="delivery-calendar" role="group" aria-label="Kalendář doručení">
            <div class="calendar-nav">
                <button type="button" aria-label="Předchozí měsíc"
                    disabled=move ||month.get() <= earliest.get().and_then(|d| d.with_day(1))
                    on:click=move |_|shift(false)>"‹"</button>
                <strong aria-live="polite">{move ||month.get().map(|d| format!("{} {}", MONTHS[d.month0() as usize], d.year()))}</strong>
                <button type="button" aria-label="Následující měsíc"
                    disabled=move || month.get().is_none_or(|d| d.year()==9999 && d.month()==12)
                    on:click=move |_|shift(true)>"›"</button>
            </div>
            <div class="calendar-grid">
                {["Po","Út","St","Čt","Pá","So","Ne"].into_iter().map(|label|view!{<span class="weekday">{label}</span>}).collect_view()}
                {move ||month.get().map(|start| {
                    let offset = start.weekday().num_days_from_monday() as usize;
                    let minimum = earliest.get();
                    let padding = (0..offset).map(|_|view!{<span aria-hidden="true"></span>}).collect_view();
                    let days = (0..31).filter_map(|i|start.checked_add_days(Days::new(i)))
                        .take_while(move |d|d.month()==start.month()).map(move |date| {
                            let selectable = selectable_date(date, minimum);
                            let value = date.to_string();
                            let selected = {
                                let value = value.clone();
                                move ||day.with(|day| *day==value)
                            };
                            view! {
                                <button type="button" disabled=!selectable aria-label=date_label(date)
                                    aria-pressed=move ||selected().to_string()
                                    on:click=move |_| {
                                        // Recheck at the interaction, even if a stale button was rendered
                                        // before midnight or its disabled attribute was changed manually.
                                        let minimum = earliest_date(earliest.get_untracked(), prague_today());
                                        if selectable_date(date, minimum) {
                                            day.set(value.clone());
                                        }
                                    }>{date.day()}</button>
                            }
                        }).collect_view();
                    (padding,days)
                })}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(value: &str) -> NaiveDate {
        value.parse().unwrap()
    }

    #[test]
    fn stale_server_dates_never_allow_today_or_the_past() {
        let first = earliest_date(Some(date("2026-11-23")), Some(date("2026-11-24")));
        assert_eq!(first, Some(date("2026-11-25")));
        assert!(!selectable_date(date("2026-11-23"), first));
        assert!(!selectable_date(date("2026-11-24"), first));
        assert!(selectable_date(date("2026-11-25"), first));
        assert!(!selectable_date(date("2026-11-28"), first));
    }

    #[test]
    fn server_floor_holidays_and_missing_options_remain_respected() {
        assert_eq!(
            earliest_date(Some(date("2026-11-30")), Some(date("2026-11-24"))),
            Some(date("2026-11-30"))
        );
        assert_eq!(
            earliest_date(Some(date("2026-12-23")), Some(date("2026-12-23"))),
            Some(date("2026-12-28"))
        );
        assert_eq!(earliest_date(None, Some(date("2026-11-24"))), None);
        assert!(!selectable_date(date("2026-11-25"), None));
    }
}
