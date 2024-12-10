use chrono::naive::NaiveDateTime;

pub fn date_format(v: NaiveDateTime) -> String {
    v.format("%Y-%m-%d %H:%M:%S").to_string()
}
