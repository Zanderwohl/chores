use chrono::{DateTime, NaiveTime, Utc, Weekday};

#[derive(Clone, PartialEq)]
pub enum ScheduleKind {
    NDays,
    NWeeks,
    Monthwise,
    WeeksOfMonth,
    CertainMonths,
    Once,
}

/// A one-time event at a specific date and time
#[derive(Clone)]
pub struct Once {
    pub datetime: DateTime<Utc>,
}

/// Every so-and-so-many days, at a certain time.
#[derive(Clone)]
pub struct NDays {
    pub days: i32,
    pub time: NaiveTime,
}

/// Every so-and-so-many weeks on certain days,
/// e.g. Every other week on Tuesdays
/// Or, every Tuesday and Thursday
#[derive(Clone)]
pub struct NWeeks {
    pub weeks: i32,
    pub sub_schedule: DaysOfWeek,
}

/// On certain days of each month, e.g. 1st and 15th
/// at a certain time
#[derive(Clone)]
pub struct Monthwise {
    pub days: Vec<i32>,
    pub time: NaiveTime,
}

/// On certain nth weekdays,
/// e.g. Every 2nd and 3rd Tuesday
/// or every Tuesday and Thursday except if it's the fifth week of the month
#[derive(Clone)]
pub struct WeeksOfMonth {
    pub weeks: Vec<i32>,
    pub sub_schedule: DaysOfWeek,
}

/// On certain days of certain months,
/// e.g. the 15th and 20th of February and March
#[derive(Clone)]
pub struct CertainMonths {
    pub months: Vec<i32>,
    pub days: Vec<i32>,
    pub time: NaiveTime,
}

#[derive(Clone)]
pub struct DaysOfWeek {
    pub sunday: bool,
    pub monday: bool,
    pub tuesday: bool,
    pub wednesday: bool,
    pub thursday: bool,
    pub friday: bool,
    pub saturday: bool,
    pub time: NaiveTime,
}

impl DaysOfWeek {
    pub fn active(&self, day: Weekday) -> bool {
        match day {
            Weekday::Sun => self.sunday,
            Weekday::Mon => self.monday,
            Weekday::Tue => self.tuesday,
            Weekday::Wed => self.wednesday,
            Weekday::Thu => self.thursday,
            Weekday::Fri => self.friday,
            Weekday::Sat => self.saturday,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_days_of_week_active() {
        let schedule = DaysOfWeek {
            sunday: true,
            monday: false,
            tuesday: true,
            wednesday: false,
            thursday: true,
            friday: false,
            saturday: true,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        };

        assert!(schedule.active(Weekday::Sun));
        assert!(!schedule.active(Weekday::Mon));
        assert!(schedule.active(Weekday::Tue));
        assert!(!schedule.active(Weekday::Wed));
        assert!(schedule.active(Weekday::Thu));
        assert!(!schedule.active(Weekday::Fri));
        assert!(schedule.active(Weekday::Sat));
    }
}
