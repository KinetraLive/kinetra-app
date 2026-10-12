// Flows: flows/<id>.json and the flow engine. Split out of the core `$lib/api` so only the
// routes that use these commands load them.

import type {
  AddFlowTemplate,
  FlowCheck,
  FlowEdge,
  FlowInfo,
  FlowNode,
  FlowTemplateInfo,
  NewFlow,
  SaveFlow,
} from "../types";
import { call } from "./index";

export const listFlows = () => call<FlowInfo[]>("list_flows");
export const getFlow = (id: string) => call<FlowInfo>("get_flow", { id });
/** Validated, written and deployed at once. */
export const createFlow = (flow: NewFlow) => call<FlowInfo>("create_flow", { flow });
/** Replaces the deployed graph (deployed at once; drops the draft). */
export const saveFlow = (flow: SaveFlow) => call<FlowInfo>("save_flow", { flow });
export const saveFlowDraft = (id: string, nodes: FlowNode[], edges: FlowEdge[]) =>
  call<FlowInfo>("save_flow_draft", { id, nodes, edges });
export const deployFlow = (id: string) => call<FlowInfo>("deploy_flow", { id });
export const discardFlowDraft = (id: string) => call<FlowInfo>("discard_flow_draft", { id });
export const renameFlow = (id: string, name: string) => call<FlowInfo>("rename_flow", { id, name });
export const duplicateFlow = (id: string) => call<FlowInfo>("duplicate_flow", { id });
export const deleteFlow = (id: string) => call<void>("delete_flow", { id });
export const setFlowArmed = (id: string, armed: boolean) =>
  call<FlowInfo>("set_flow_armed", { id, armed });
/** Runs a flow from its Cue trigger; progress via the `flow-state` event. */
export const runFlow = (id: string, value?: string) =>
  call<void>("run_flow", { id, value: value ?? null });
export const stopFlow = (id: string) => call<void>("stop_flow", { id });
/** Validation without saving: errors block deploy, warnings don't. With `id`, a Run flow of the
 * flow itself is reported too ("A flow can't run itself; use Repeat"). */
export const checkFlow = (nodes: FlowNode[], edges: FlowEdge[], id?: string) =>
  call<FlowCheck>("check_flow", { nodes, edges, id: id ?? null });
/** The editor shows this flow: `flow-debug` batches arrive until unsubscribed (counted). */
export const subscribeFlowDebug = (id: string) => call<void>("subscribe_flow_debug", { id });
export const unsubscribeFlowDebug = (id: string) => call<void>("unsubscribe_flow_debug", { id });
/** Built-in flow templates (gallery order). */
export const listFlowTemplates = () => call<FlowTemplateInfo[]>("list_flow_templates");
/** Adds a template pointed at the chosen slots; returns the created flows. */
export const addFlowTemplate = (req: AddFlowTemplate) =>
  call<FlowInfo[]>("add_flow_template", { req });
