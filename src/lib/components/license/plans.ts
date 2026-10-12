// Display names for license plans and states (display only; nothing is gated on them).

const PLANS: Readonly<Record<string, string>> = {
  free: "Free",
  beta: "Beta",
  pro_monthly: "Pro (monthly)",
  pro_yearly: "Pro (yearly)",
  pro_perpetual: "Pro (perpetual)",
  day_pass: "Day pass",
  enterprise: "Enterprise",
  nfr: "Not for resale",
  nonprofit: "Nonprofit",
};

export const planName = (plan: string | null | undefined) =>
  !plan ? "No license" : (PLANS[plan] ?? plan.replace(/_/g, " ").replace(/^\w/, (c) => c.toUpperCase()));

const STATUS: Readonly<Record<string, string>> = {
  none: "No license on this computer",
  active: "Active",
  reduced: "Limited until Kinetra connects",
  offlineExpired: "Expired: offline too long",
  expired: "Ended",
  notCovered: "Doesn't cover this version",
  revoked: "Revoked",
  clockTampered: "Paused: the computer's clock is behind",
};

export const statusText = (s: string) => STATUS[s] ?? s;

/** Problem states (shown in the warning color). */
export const statusIsProblem = (s: string) => !["active", "none"].includes(s);

/** "12 Mar 2027" from unix seconds. */
export const dateText = (unix: number) =>
  new Date(unix * 1000).toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });

/** "3 min ago" from unix seconds (body font, not mono). */
export function agoText(unix: number, now = Date.now() / 1000): string {
  const s = Math.max(0, Math.round(now - unix));
  if (s < 60) return "Just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return h === 1 ? "1 hour ago" : `${h} hours ago`;
  const d = Math.round(h / 24);
  return `${d} days ago`;
}
