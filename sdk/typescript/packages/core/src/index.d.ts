/** @packageDocumentation Upstream-first v2 pipeline authoring declarations. */

export type {
  AuthoringConfigFor,
  BoundaryInterface,
  BoundaryTypeExpression,
  CanonicalActorTypeFor,
  EmaConfig,
  GeneratedConfigFor,
  InputConfig,
  PipelineActorConfig,
  PredicateExpression,
  ProjectInputConfig,
  ProjectOutputConfig,
  PublicActorSpelling,
  ReplacementConfig,
  ReplicatorConfig,
  JsonConfig,
  TemplateReferenceMap,
  TransformExpression,
} from "./config.js";

export type {
  AlignmentRelation,
  AuthoredAnchor,
  AuthoredPoint,
  AuthoredSize,
  BoardPlacement,
  FlowLayout,
  GridFlowOptions,
  LayoutActorReference,
  LinearFlowOptions,
  ActorAuthoringSurface,
  ActorFlags,
  Presentation,
  RelativeRelation,
  StaticViewSelection,
  ViewKind,
  ViewSelection,
} from "./presentation.js";
export { flow } from "./presentation.js";
export type {
  ViewFieldPath,
  ViewFieldRole,
  ViewTotal,
  ViewVocabulary,
  ViewVocabularyKey,
  ViewConfig,
  ViewConfigValue,
  ConfigSlotMetadata,
} from "./view-config.js";
export { validateViewConfig, resolveViewConfig } from "./view-config.js";
export { declareActor, declareEdge, declareScope, moveToScope, setFlags } from "./structure.js";

export type {
  CurrentEdgeHandle,
  CurrentHandleMode,
  DeclaredDelay,
  DeliveryPolicy,
  EdgeOptions,
  HandleMode,
  NewEdgeHandle,
  NewHandleMode,
  ObservedEndpoint,
  ShedPolicy,
  SourceEndpoint,
  PendingEndpoint,
  TargetEndpoint,
  WirePolicy,
  WritableBoundaryEndpoint,
} from "./wiring.js";

export type {
  CurrentFlowActorHandle,
  CurrentMultiOutletActorHandle,
  CurrentActorHandle,
  CurrentActorOperations,
  CurrentSourceActorHandle,
  CurrentTerminalActorHandle,
  CurrentWritableBoundaryActorHandle,
  FlowActorHandle,
  FlowActorSpelling,
  DynamicPortActorSpelling,
  MultiOutletActorHandle,
  MultiOutletActorSpelling,
  NewFlowActorHandle,
  NewMultiOutletActorHandle,
  NewActorHandle,
  NewSourceActorHandle,
  NewTerminalActorHandle,
  NewWritableBoundaryActorHandle,
  ActorHandle,
  PipelineActorHandle,
  ReplicatorActorHandle,
  SourceActorHandle,
  SourceOnlyActorSpelling,
  TerminalActorHandle,
  TerminalActorSpelling,
  WritableBoundaryActorHandle,
} from "./handles.js";

export type { DownstreamActorMethods } from "./catalog.js";
export { route, pipelineActor, debounce, alert, tap, input, output, replicator, agent, counter, ema, windowedReduce, timer, toolExecutor, notify, peer, listener, keyedReduce, request, file, json, otlp, match, assemble, join, form, projectInput, projectOutput, merge } from "./catalog.js";

/** An authored note uses the same presentation verbs as actors. */
export declare function note(config: { readonly id?: string; readonly refs?: readonly import("./presentation.js").LayoutActorReference[]; readonly text: string }): Omit<import("./presentation.js").ActorAuthoringSurface, "setFlags">;
