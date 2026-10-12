// Minimal DOM matchers for the templates tests (the repo has no jest-dom).

import { expect } from "vitest";

interface DomMatchers<R = unknown> {
  toBeInTheDocument(): R;
  toHaveAttribute(name: string, value?: string): R;
  toHaveTextContent(text: string | RegExp): R;
  toBeDisabled(): R;
}

declare module "vitest" {
  interface Matchers<R extends void | Promise<void> = void | Promise<void>, T = unknown>
    extends DomMatchers<R> {}
}

const el = (x: unknown): Element | null => (x instanceof Element ? x : null);

expect.extend({
  toBeInTheDocument(received: unknown) {
    const e = el(received);
    const pass = !!e && e.ownerDocument.contains(e);
    return { pass, message: () => `expected element ${pass ? "not " : ""}to be in the document` };
  },
  toHaveAttribute(received: unknown, name: string, value?: string) {
    const actual = el(received)?.getAttribute(name) ?? null;
    const pass = value === undefined ? actual !== null : actual === value;
    return {
      pass,
      message: () =>
        `expected [${name}]${value === undefined ? "" : `="${value}"`} ${pass ? "not " : ""}to be present (got ${JSON.stringify(actual)})`,
    };
  },
  toHaveTextContent(received: unknown, text: string | RegExp) {
    const actual = (el(received)?.textContent ?? "").replace(/\s+/g, " ").trim();
    const pass = typeof text === "string" ? actual.includes(text) : text.test(actual);
    return { pass, message: () => `expected text ${JSON.stringify(actual)} ${pass ? "not " : ""}to match ${text}` };
  },
  toBeDisabled(received: unknown) {
    const e = el(received);
    const pass = !!e && (e as HTMLButtonElement).disabled === true;
    return { pass, message: () => `expected element ${pass ? "not " : ""}to be disabled` };
  },
});
