// Sign-in and licensing. The core decides everything; these only start actions or read the view.

import type { CloudLicense, LicenseView, SigninView } from "../types";
import { call } from "./index";

export const licenseView = () => call<LicenseView>("license_view");
/** Starts the device flow; progress arrives as `license-changed` (`signin`). */
export const signInStart = () => call<SigninView>("sign_in_start");
export const signInCancel = () => call<void>("sign_in_cancel");
export const signOut = () => call<void>("sign_out");
export const licenseActivate = (slot: "main" | "backup", licenseId: string | null = null) =>
  call<void>("license_activate", { slot, licenseId });
export const licenseDeactivate = (activationId: string) => call<void>("license_deactivate", { activationId });
/** "Move Pro here": deactivates `activationId` (another computer), then refreshes here. */
export const licenseMoveHere = (activationId: string) => call<void>("license_move_here", { activationId });
export const licenseRedeem = (key: string) => call<void>("license_redeem", { key });
export const licenseList = () => call<CloudLicense[]>("license_list");
export const licenseCheckNow = () => call<void>("license_check_now");
export const cloudLinks = () => call<{ account: string; pricing: string; download: string }>("cloud_links");
