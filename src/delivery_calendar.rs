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
                if let Some(next) = next {
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
                            let selectable = minimum.is_some_and(|first|date >= first) && working_day(date);
                            let value = date.to_string();
                            let selected = {
                                let value = value.clone();
                                move ||day.with(|day| *day==value)
                            };
                            view! {
                                <button type="button" disabled=!selectable aria-label=date_label(date)
                                    aria-pressed=move ||selected().to_string()
                                    on:click=move |_|day.set(value.clone())>{date.day()}</button>
                            }
                        }).collect_view();
                    (padding,days)
                })}
            </div>
        </div>
    }
}
