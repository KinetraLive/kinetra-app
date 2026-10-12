// The OS the app runs on, for platform words in the UI (menus, the secret store).

export const isMac = (): boolean => typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

/** Where the OS keeps secret header values (the keyring backend the core uses). */
export const secretStoreName = (mac = isMac()): string => (mac ? "macOS Keychain" : "Windows Credential Manager");
