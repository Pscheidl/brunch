use chrono::{Datelike, Days, NaiveDate, Weekday};

fn easter_sunday(year: i32) -> NaiveDate {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32).expect("Gregorian Easter date")
}

pub fn working_day(date: NaiveDate) -> bool {
    let holiday = matches!(
        (date.month(), date.day()),
        (1, 1)
            | (5, 1)
            | (5, 8)
            | (7, 5)
            | (7, 6)
            | (9, 28)
            | (10, 28)
            | (11, 17)
            | (12, 24)
            | (12, 25)
            | (12, 26)
    );
    let easter = easter_sunday(date.year());
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
        && !holiday
        && Some(date) != easter.checked_sub_days(Days::new(2))
        && Some(date) != easter.checked_add_days(Days::new(1))
}
