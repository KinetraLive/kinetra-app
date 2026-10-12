// The OS the app runs on, for platform words in the UI (menus, file manager, firewall, secrets).

export const isMac = (): boolean => typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

/** The file manager's "show this file" action. */
export const revealLabel = (mac = isMac()): string => (mac ? "Show in Finder" : "Show in Explorer");

/** Turning on LAN access: Windows Firewall asks once; macOS has no such prompt by default. */
export const lanAccessNote = (mac = isMac()): string =>
  mac ? "Other devices can view outputs." : "Other devices can view outputs. Windows may ask once.";

/** Where the OS keeps secret header values (the keyring backend the core uses). */
export const secretStoreName = (mac = isMac()): string => (mac ? "macOS Keychain" : "Windows Credential Manager");
