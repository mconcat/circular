#![forbid(unsafe_code)]
#![doc = ""]

mod admission;
mod identity;
mod model;
mod scope;
mod spelling;
mod value;

pub use admission::{
    AdmittedScope, AdmittedTemplate, CellDerivationError, InstanceBinding, ScopeAdmissionError,
    admit_runtime_scope, admit_template,
};
pub use circular_core::{ActorType, PortId};
pub use identity::{
    ActorId, Generation, GenerationVector, GenerationVectorError, Incarnation, InstanceKey,
    InstanceScalar, LocalKey, MAX_SCOPE_DEPTH, Name, NamedActorId, ScopeId, ScopeIdError, ScopeSeg,
    ScopedActorId, SystemActor, Uuid,
};
pub use model::{
    Annotation, AnnotationId, AnnotationKind, BoundaryPortRef, DeclaredEdgeId, DeclaredScopeSeg,
    Export, ExportName, Mount, Role,
};
pub use scope::{ScopeBoundary, ScopeDeclaration, ScopeRole, ScopeRoleTable};
pub use value::{
    ActorDecl, ActorDomain, ActorFlags, Anchor, AnnotationPlacement, Axis, BoardPlacement,
    BoardPlacementError, Config, ConfigError, ConfigRecord, ConfigValue, ConfigValueError,
    ContainerActorDecl, DeclaredDelay, DeclaredDelayDurationError, DeclaredDelayError, Delivery,
    EdgeAttrs, EdgeDecl, EdgeId, Endpoint, GroupName, GroupNameError, LayoutCoord, LayoutPoint,
    LayoutSize, NonContainerActorDecl, NonContainerActorDeclError, OperationDecl,
    PipelineActorDecl, PositiveCapacity, PositiveCapacityError, PreprocessChain, PreprocessKind,
    PreprocessStep, Presentation, Relation, Shed, Text, ViewSpec, WirePolicy,
};

pub mod template;
pub use template::Template;
