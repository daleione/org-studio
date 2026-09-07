use jiff::civil::Date;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HabitStats {
    pub(crate) completed: u8,
    pub(crate) current_streak: u8,
    pub(crate) best_streak: u8,
    pub(crate) completion_percent: u8,
}
pub(crate) fn habit_stats(today: Date, completions: impl IntoIterator<Item = Date>) -> HabitStats {
    let completed = completions.into_iter().collect::<BTreeSet<_>>();
    let mut days = Vec::with_capacity(28);
    let mut day = today;
    for _ in 0..28 {
        days.push(day);
        day = day.yesterday().unwrap_or(day);
    }
    let count = days.iter().filter(|day| completed.contains(day)).count() as u8;
    let current = days
        .iter()
        .take_while(|day| completed.contains(day))
        .count() as u8;
    let (mut best, mut run) = (0u8, 0u8);
    for day in days.iter().rev() {
        if completed.contains(day) {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    HabitStats {
        completed: count,
        current_streak: current,
        best_streak: best,
        completion_percent: ((u16::from(count) * 100) / 28) as u8,
    }
}
