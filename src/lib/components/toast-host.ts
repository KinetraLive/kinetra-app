// The root layout's lazy ToastHost, behind a module with no static imports: the layout (in the
// picker's initial bundle) then carries one preload entry instead of ToastHost's whole list.
export const loadToastHost = () => import("./ToastHost.svelte");
