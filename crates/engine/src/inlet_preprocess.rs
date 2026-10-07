//! Edge programs compiled during flattening, shared by live inlets and replay prediction.
use std::sync::Arc;

use circular_actors::{CompiledDecoder, ParseConfig, ProductPayload};
use circular_expr::snippet::Snippet;
use circular_plan::{EdgeId, PreprocessChain, PreprocessKind};
use circular_runtime::ProcessingCause;

use crate::run_graph::GraphError;

#[derive(Debug)]
pub enum CompiledStep {
    Map(Snippet),
    Filter(Snippet),
    Bang,
    Parse(ParseConfig, CompiledDecoder),
    Flatten(circular_actors::flatten::FlattenConfig),
}

impl CompiledStep {
    /// Wire type rules, preserving the former unary flow algebra. Map's
    /// item is inferred by the same Snippet that evaluates its payload.
    pub(crate) fn flows(&self) -> (circular_actors::Flow, circular_actors::Flow) {
        use circular_actors::{BaseShape, FieldMap, Flow, Name, Shape};
        let variable = Shape::Var(Name::from_static("T"));
        let (input, output) = match self {
            Self::Map(snippet) => {
                let output = circular_actors::types::from_unnamed_shape(
                    &snippet.output_shape(&circular_actors::map_config::shape_env(&variable)),
                );
                (variable, output)
            }
            Self::Filter(_) => (variable.clone(), variable),
            Self::Bang => (Shape::Any, Shape::Base(BaseShape::Null)),
            Self::Flatten(..) => (
                Shape::Any,
                Shape::Object {
                    fields: FieldMap::try_new(Vec::new()).unwrap(),
                    open: true,
                },
            ),
            Self::Parse(..) => {
                let object = Shape::Object {
                    fields: FieldMap::try_new(Vec::new()).expect("empty field map is unique"),
                    open: true,
                };
                (object.clone(), object)
            }
        };
        (Flow::Stream(input), Flow::Stream(output))
    }
}

#[derive(Clone, Debug, Default)]
pub struct CompiledPreprocess {
    source: Arc<PreprocessChain>,
    steps: Arc<[Arc<CompiledStep>]>,
    failure_points: Arc<[circular_runtime::PreprocessFailurePoint]>,
}

impl PartialEq for CompiledPreprocess {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && self.failure_points == other.failure_points
    }
}
impl Eq for CompiledPreprocess {}

#[cfg(test)]
thread_local! { pub(crate) static COMPILE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

impl CompiledPreprocess {
    pub(crate) fn compile(edge: &EdgeId, chain: &PreprocessChain) -> Result<Self, GraphError> {
        #[cfg(test)]
        COMPILE_COUNT.with(|count| count.set(count.get() + usize::from(!chain.steps().is_empty())));
        let mut steps = Vec::new();
        for (index, step) in chain.steps().iter().enumerate() {
            let compile = || -> Result<CompiledStep, String> {
                let value = crate::activation_config::fold_preprocess_config(step.config())?;
                Ok(match step.kind() {
                    PreprocessKind::Map => CompiledStep::Map(
                        circular_actors::accept_transform(&value).map_err(|e| e.to_string())?,
                    ),
                    PreprocessKind::Filter => CompiledStep::Filter(
                        circular_actors::accept_predicate(&value).map_err(|e| e.to_string())?,
                    ),
                    PreprocessKind::Bang => {
                        circular_actors::reject_nonempty_config(&value)
                            .map_err(|e| e.to_string())?;
                        CompiledStep::Bang
                    }
                    PreprocessKind::Flatten => CompiledStep::Flatten(
                        circular_actors::flatten::FlattenConfig::from_value(&value)?,
                    ),
                    PreprocessKind::Parse => {
                        let config = ParseConfig::from_value(&value).map_err(|e| e.to_string())?;
                        let decoder = CompiledDecoder::compile(config.decoder())
                            .map_err(|e| e.to_string())?;
                        CompiledStep::Parse(config, decoder)
                    }
                })
            };
            steps.push(Arc::new(compile().map_err(|detail| {
                GraphError::PreprocessInvariant {
                    edge: Box::new(edge.clone()),
                    step: index,
                    detail: crate::activation_config::config_rejection(
                        match edge {
                            EdgeId::Declared { to, .. } => to.actor(),
                            EdgeId::Outcome { target } => target,
                        },
                        &format!("preprocess[{index}].config"),
                        step.config(),
                        detail,
                    ),
                }
            })?));
        }
        Ok(Self {
            source: Arc::new(chain.clone()),
            steps: steps.into(),
            failure_points: chain
                .steps()
                .iter()
                .enumerate()
                .map(|(index, step)| circular_runtime::PreprocessFailurePoint {
                    edge: edge.clone(),
                    index,
                    kind: step.kind(),
                    code: None,
                })
                .collect(),
        })
    }

    /// Bind the authored instruction locations to this routed edge's canonical coordinates.
    pub(crate) fn for_edge(&self, edge: &EdgeId) -> Self {
        let mut program = self.clone();
        program.failure_points = self
            .failure_points
            .iter()
            .cloned()
            .map(|mut point| {
                point.edge = edge.clone();
                point
            })
            .collect();
        program
    }

    pub(crate) fn then(&self, next: &Self) -> Self {
        Self {
            source: Arc::new(PreprocessChain::new(
                self.source
                    .steps()
                    .iter()
                    .chain(next.source.steps())
                    .cloned()
                    .collect::<Vec<_>>(),
            )),
            failure_points: self
                .failure_points
                .iter()
                .chain(next.failure_points.iter())
                .cloned()
                .collect(),
            steps: self
                .steps
                .iter()
                .chain(next.steps.iter())
                .cloned()
                .collect(),
        }
    }

    /// Err skips the remaining program, preserving the value the failed step received and the
    /// first reason. This is rerun from each raw recorded arrival, in its admitted revision.
    pub(crate) fn apply<D: InletPreprocess + Clone>(
        &self,
        input: &D,
    ) -> InletVerdict<(D, circular_runtime::EnvelopeResult)> {
        let mut values = vec![(input.clone(), circular_runtime::EnvelopeResult::Ok)];
        for (index, step) in self.steps().iter().enumerate() {
            let mut next = Vec::new();
            for (payload, tag) in values {
                if matches!(tag, circular_runtime::EnvelopeResult::Err { .. }) {
                    next.push((payload, tag));
                    continue;
                }
                match payload.apply(step) {
                    StepVerdict::Emitted(value) => next.push((value, tag)),
                    StepVerdict::Batch(batch) => {
                        next.extend(batch.into_iter().map(|v| (v, tag.clone())))
                    }
                    StepVerdict::Suppressed => {}
                    StepVerdict::Failed(detail) => next.push((
                        payload,
                        circular_runtime::EnvelopeResult::Err {
                            reason: circular_runtime::DeadLetterReason::Processing(
                                ProcessingCause::InputOutOfDomain,
                            ),
                            failure_point: self.failure_points.get(index).cloned().map(
                                |mut point| {
                                    point.code = Some(detail.code().to_owned());
                                    point
                                },
                            ),
                        },
                    )),
                }
            }
            values = next;
        }
        if values.is_empty()
            && !self
                .steps()
                .iter()
                .any(|step| matches!(step.as_ref(), CompiledStep::Flatten(_)))
        {
            return InletVerdict::Suppressed;
        }
        InletVerdict::Elements(values)
    }

    pub(crate) fn apply_result_to_inlet<D: InletPreprocess + Clone>(
        &self,
        input: &D,
        result: &circular_runtime::EnvelopeResult,
        actor_type: circular_core::ActorType,
        inlet: &circular_core::PortId,
    ) -> InletVerdict<Element<D>> {
        let evaluated = match result {
            circular_runtime::EnvelopeResult::Ok => self.apply(input),
            circular_runtime::EnvelopeResult::Err { .. } => {
                InletVerdict::Elements(vec![(input.clone(), result.clone())])
            }
        };
        let consumes_err =
            actor_type == circular_core::ActorType::Match && inlet.as_str() == "event";
        match evaluated {
            InletVerdict::Suppressed => InletVerdict::Suppressed,
            InletVerdict::Elements(values) => InletVerdict::Elements(
                values
                    .into_iter()
                    .map(|(payload, result)| match result {
                        circular_runtime::EnvelopeResult::Err {
                            reason: circular_runtime::DeadLetterReason::Processing(cause),
                            failure_point,
                        } if !consumes_err => Element::Failed {
                            subject: payload,
                            cause,
                            point: failure_point,
                        },
                        result => Element::Input(payload, result),
                    })
                    .collect(),
            ),
        }
    }

    pub(crate) fn steps(&self) -> &[Arc<CompiledStep>] {
        &self.steps
    }
}

pub enum InletVerdict<E> {
    Elements(Vec<E>),
    Suppressed,
}

pub enum Element<D> {
    Input(D, circular_runtime::EnvelopeResult),
    Failed {
        subject: D,
        cause: ProcessingCause,
        point: Option<circular_runtime::PreprocessFailurePoint>,
    },
}

pub enum StepVerdict<D> {
    Emitted(D),
    Batch(Vec<D>),
    Suppressed,
    Failed(circular_actors::FailureDetail),
}

pub trait InletPreprocess: Sized {
    fn apply(&self, step: &CompiledStep) -> StepVerdict<Self>;
}

impl InletPreprocess for ProductPayload {
    fn apply(&self, step: &CompiledStep) -> StepVerdict<Self> {
        use crate::activation_detail::preprocess;
        match step {
            CompiledStep::Flatten(config) => match config.expand(self) {
                Ok(values) => StepVerdict::Batch(values),
                Err(failure) => StepVerdict::Failed(preprocess::flatten(failure)),
            },
            CompiledStep::Map(snippet) => match circular_actors::map_event(snippet, self) {
                Ok(payload) => StepVerdict::Emitted(payload),
                Err(failure) => StepVerdict::Failed(preprocess::map(&failure)),
            },
            CompiledStep::Filter(snippet) => match circular_actors::filter_event(snippet, self) {
                Ok(true) => StepVerdict::Emitted(self.clone()),
                Ok(false) => StepVerdict::Suppressed,
                Err(failure) => StepVerdict::Failed(preprocess::filter(&failure)),
            },
            CompiledStep::Bang => StepVerdict::Emitted(circular_actors::bang_event()),
            CompiledStep::Parse(config, decoder) => {
                match circular_actors::parse_event(config, decoder, self) {
                    Ok(payload) => StepVerdict::Emitted(payload),
                    Err(failure) => StepVerdict::Failed(preprocess::parse(&failure)),
                }
            }
        }
    }
}

#[cfg(test)]
macro_rules! identity {
    ($($ty:ty),*) => { $(impl InletPreprocess for $ty {
        fn apply(&self, _: &CompiledStep) -> StepVerdict<Self> { StepVerdict::Emitted(self.clone()) }
    })* };
}
#[cfg(test)]
identity!(i64, (), &str, circular_core::Value);
#[cfg(test)]
impl<N: Clone> InletPreprocess for circular_core::Payload<N, i64> {
    fn apply(&self, _: &CompiledStep) -> StepVerdict<Self> {
        StepVerdict::Emitted(self.clone())
    }
}

#[cfg(test)]
mod result_tests {
    use super::*;
    use circular_actors::{GroundShape, Shape};
    use circular_core::{ActorType, Value};
    use circular_runtime::{DeadLetterReason, EnvelopeResult};

    fn program(steps: Vec<CompiledStep>) -> CompiledPreprocess {
        CompiledPreprocess {
            source: Arc::default(),
            steps: steps.into_iter().map(Arc::new).collect(),
            failure_points: Arc::default(),
        }
    }
    fn map(text: &str) -> CompiledStep {
        CompiledStep::Map(
            circular_actors::accept_transform(
                &Value::object([("transform", Value::string(text))]).unwrap(),
            )
            .unwrap(),
        )
    }
    fn filter(text: &str) -> CompiledStep {
        CompiledStep::Filter(
            circular_actors::accept_predicate(
                &Value::object([("predicate", Value::string(text))]).unwrap(),
            )
            .unwrap(),
        )
    }
    fn parse() -> CompiledStep {
        let config = ParseConfig::from_value(
            &Value::object([
                ("decoder", Value::string("json")),
                ("field", Value::string("raw")),
            ])
            .unwrap(),
        )
        .unwrap();
        let decoder = CompiledDecoder::compile(config.decoder()).unwrap();
        CompiledStep::Parse(config, decoder)
    }
    fn payload(value: Value) -> ProductPayload {
        ProductPayload::new(GroundShape::try_new(Shape::Any).unwrap(), value)
    }

    fn flatten(at: &str) -> CompiledStep {
        CompiledStep::Flatten(
            circular_actors::flatten::FlattenConfig::from_value(
                &Value::object([("at", Value::array([Value::string(at)]))]).unwrap(),
            )
            .unwrap(),
        )
    }
    fn object(entries: &[(&str, Value)]) -> Value {
        Value::object(entries.iter().cloned()).unwrap()
    }

    #[test]
    fn result_failures_skip_the_remaining_chain_and_preserve_the_first_error() {
        let input = payload(
            Value::object([("raw", Value::string("{")), ("exit", Value::uint(0))]).unwrap(),
        );
        for step in [parse(), map("event.missing"), filter("event.exit != 0")] {
            let chain = program(vec![
                step,
                CompiledStep::Bang,
                filter("false"),
                map("event.missing"),
            ]);
            let InletVerdict::Elements(elements) = chain.apply(&input) else {
                panic!("failure must be an envelope");
            };
            let [(actual, result)] = elements.as_slice() else {
                panic!("one element: {}", elements.len());
            };
            assert_eq!(actual, &input, "no later transform may touch Err");
            assert_eq!(
                result,
                &EnvelopeResult::Err {
                    reason: DeadLetterReason::Processing(ProcessingCause::InputOutOfDomain),
                    failure_point: None,
                }
            );
        }
    }
    #[test]
    fn result_false_is_suppressed_and_success_keeps_its_payload_type() {
        assert!(matches!(
            program(vec![filter("false"), map("event.missing")]).apply(&payload(Value::int(1))),
            InletVerdict::Suppressed
        ));
        let input = payload(Value::object([("raw", Value::string("{\"n\":21}"))]).unwrap());
        let InletVerdict::Elements(elements) =
            program(vec![parse(), map("event.n * 2")]).apply(&input)
        else {
            panic!("success");
        };
        let [(output, result)] = elements.as_slice() else {
            panic!("one element: {}", elements.len());
        };
        assert_eq!(output.value(), &Value::int(42));
        assert_eq!(result, &EnvelopeResult::Ok);
    }

    fn split(
        chain: &CompiledPreprocess,
        input: &ProductPayload,
        result: &EnvelopeResult,
        actor_type: ActorType,
        inlet: &circular_core::PortId,
    ) -> (Vec<Value>, Vec<Value>) {
        let InletVerdict::Elements(elements) =
            chain.apply_result_to_inlet(input, result, actor_type, inlet)
        else {
            panic!("an expansion settles as elements");
        };
        let mut inputs = Vec::new();
        let mut failures = Vec::new();
        for element in elements {
            match element {
                Element::Input(payload, result) => {
                    assert_eq!(result, EnvelopeResult::Ok);
                    inputs.push(payload.value().clone());
                }
                Element::Failed { subject, cause, .. } => {
                    assert_eq!(cause, ProcessingCause::InputOutOfDomain);
                    failures.push(subject.value().clone());
                }
            }
        }
        (inputs, failures)
    }
}
