pub trait AlarmFunctions {
    /// Ignore all alarms current matched by the filter. If permanent
    /// is false, the alarms will be restored when they no longer
    /// match.
    fn ignore_matched_alarms(&self, filter: &str, permanent: bool);

    /// Stop ignoring alarms for the filter.
    fn restore_ignored_alarms(&self, filter: &str);
}
