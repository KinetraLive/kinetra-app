// The shell's lazy Simple-mode view of /flows (SimpleGate), behind a module with no static
// imports: the shell then carries one preload entry instead of FlowsHidden's whole list.
export const loadFlowsHidden = () => import("./FlowsHidden.svelte");
