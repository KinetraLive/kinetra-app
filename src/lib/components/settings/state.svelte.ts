// Window-wide state shared by the Preferences sections.

/** While a move runs, the page's other destructive actions are disabled. */
export const settingsBusy = $state({ moving: false });

/** Bumped to ask About to check for updates (Help › Check for updates, `?section=updates`); About
 * runs one check per bump, also when it mounts after the bump. */
export const updateRequest = $state({ seq: 0 });
