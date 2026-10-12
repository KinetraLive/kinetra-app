// A trampoline with no static imports: ./eager.svelte.ts imports this module (so its preload list
// is one file), and this one loads the heavy chunks with their own preload lists.

export const host = () => import("./host.svelte");
export const store = () => import("./controls.svelte");
