
use super::annotation::{PlanAnnotationKey, decode_annotation_address};
use circular_core::{Ceilings, Value, decode};
use std::fmt;

#[cfg(test)]
use crate::scope_identity::ScopeSegment;
use crate::scope_identity::{
    AddressContext, AddressRef, PlanActorKey, decode_actor_address, decode_plan_actor_key,
};
use crate::wire_value::{
    PayloadRejection, bool_of, decode_arm, exhausted, int_of, object_fields, object_from_value,
    optional, take, text_of, unsigned_of,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ViewSpec {
    pub kind: String,
    pub config: Value,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BoardPlacement {
    col: u32,
    row: u32,
    w: u32,
    h: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoardPlacementError {
    ZeroExtent { w: u32, h: u32 },
    ExtentOverflow { start: u32, span: u32 },
}

impl BoardPlacement {
    pub const fn try_new(col: u32, row: u32, w: u32, h: u32) -> Result<Self, BoardPlacementError> {
        if w == 0 || h == 0 {
            return Err(BoardPlacementError::ZeroExtent { w, h });
        }
        if col.checked_add(w).is_none() {
            return Err(BoardPlacementError::ExtentOverflow {
                start: col,
                span: w,
            });
        }
        if row.checked_add(h).is_none() {
            return Err(BoardPlacementError::ExtentOverflow {
                start: row,
                span: h,
            });
        }
        Ok(Self { col, row, w, h })
    }

    #[must_use]
    pub const fn col(self) -> u32 {
        self.col
    }

    #[must_use]
    pub const fn row(self) -> u32 {
        self.row
    }

    #[must_use]
    pub const fn w(self) -> u32 {
        self.w
    }

    #[must_use]
    pub const fn h(self) -> u32 {
        self.h
    }

    #[must_use]
    pub const fn col_end(self) -> u32 {
        self.col + self.w
    }

    #[must_use]
    pub const fn row_end(self) -> u32 {
        self.row + self.h
    }

    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        self.col < other.col_end()
            && other.col < self.col_end()
            && self.row < other.row_end()
            && other.row < self.row_end()
    }
}

impl fmt::Display for BoardPlacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "(col {}, row {}, {}\u{00d7}{})",
            self.col, self.row, self.w, self.h
        )
    }
}

impl fmt::Display for BoardPlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroExtent { w, h } => write!(
                formatter,
                "board cell occupies no cell ({w}\u{00d7}{h}); an extent of zero is not a placement"
            ),
            Self::ExtentOverflow { start, span } => write!(
                formatter,
                "board cell runs past the grid ({start} + {span} exceeds u32)"
            ),
        }
    }
}

impl std::error::Error for BoardPlacementError {}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GroupName(Box<str>);

impl GroupName {
    pub fn from_authored_syntax(bytes: &[u8]) -> Result<Self, GroupNameError> {
        let authored =
            std::str::from_utf8(bytes).map_err(|_| GroupNameError::InvalidUtf8Carrier)?;
        let normalized = icu_normalizer::ComposingNormalizerBorrowed::new_nfc()
            .normalize(authored)
            .into_owned();

        if normalized.is_empty() {
            return Err(GroupNameError::Empty);
        }
        if normalized.len() > 64 {
            return Err(GroupNameError::TooLong {
                bytes: normalized.len(),
            });
        }
        if normalized.chars().any(|character| {
            matches!(
                character,
                '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}'
            )
        }) {
            return Err(GroupNameError::ControlCharacter);
        }
        if normalized.chars().next().is_some_and(char::is_whitespace)
            || normalized
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            return Err(GroupNameError::SurroundingWhitespace);
        }

        Ok(Self(normalized.into_boxed_str()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupNameError {
    InvalidUtf8Carrier,
    Empty,
    TooLong { bytes: usize },
    ControlCharacter,
    SurroundingWhitespace,
}

impl fmt::Display for GroupNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8Carrier => formatter.write_str("group name is not a UTF-8 string"),
            Self::Empty => formatter.write_str("group name is empty after NFC normalization"),
            Self::TooLong { bytes } => write!(
                formatter,
                "group name is {bytes} bytes after NFC normalization (maximum 64)"
            ),
            Self::ControlCharacter => {
                formatter.write_str("group name contains a C0/C1 control character")
            }
            Self::SurroundingWhitespace => {
                formatter.write_str("group name has leading or trailing whitespace")
            }
        }
    }
}

impl std::error::Error for GroupNameError {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LayoutPoint {
    pub x: LayoutCoord,
    pub y: LayoutCoord,
}

impl LayoutPoint {
    #[must_use]
    pub const fn new(x: LayoutCoord, y: LayoutCoord) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn x(self) -> LayoutCoord {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> LayoutCoord {
        self.y
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayoutCoord(i32);

impl LayoutCoord {
    #[must_use]
    pub const fn new(units: i32) -> Self {
        Self(units)
    }

    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LayoutSize {
    pub w: u64,
    pub h: u64,
}

impl LayoutSize {
    #[must_use]
    pub const fn new(w: u64, h: u64) -> Self {
        Self { w, h }
    }

    #[must_use]
    pub const fn width(self) -> u64 {
        self.w
    }

    #[must_use]
    pub const fn height(self) -> u64 {
        self.h
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Anchor<A> {
    Flow,
    Relative { target: A, relation: Relation },
    Align { target: A, axis: Axis },
}

circular_core::closed_table! {
    pub enum Relation: i64 {
        Before = 1,
        After = 2,
    }
}

circular_core::closed_table! {
    pub enum Axis: i64 {
        Horizontal = 1,
        Vertical = 2,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Presentation<A> {
    pub label: Option<String>,
    pub group: Option<GroupName>,
    pub anchor: Option<Anchor<A>>,
    pub fixed: Option<LayoutPoint>,
    pub size: Option<LayoutSize>,
    pub board: Option<BoardPlacement>,
    pub view: Option<ViewSpec>,
    pub collapsed: bool,
}

impl<A> Default for Presentation<A> {
    fn default() -> Self {
        Self {
            label: None,
            group: None,
            anchor: None,
            fixed: None,
            size: None,
            board: None,
            view: None,
            collapsed: false,
        }
    }
}

impl<A> Presentation<A> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        label: Option<String>,
        group: Option<GroupName>,
        anchor: Option<Anchor<A>>,
        fixed: Option<LayoutPoint>,
        size: Option<LayoutSize>,
        board: Option<BoardPlacement>,
        view: Option<ViewSpec>,
        collapsed: bool,
    ) -> Self {
        Self {
            label,
            group,
            anchor,
            fixed,
            size,
            board,
            view,
            collapsed,
        }
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    #[must_use]
    pub const fn group(&self) -> Option<&GroupName> {
        self.group.as_ref()
    }

    #[must_use]
    pub const fn anchor(&self) -> Option<&Anchor<A>> {
        self.anchor.as_ref()
    }

    #[must_use]
    pub const fn fixed(&self) -> Option<LayoutPoint> {
        self.fixed
    }

    #[must_use]
    pub const fn size(&self) -> Option<LayoutSize> {
        self.size
    }

    #[must_use]
    pub const fn board(&self) -> Option<BoardPlacement> {
        self.board
    }

    #[must_use]
    pub const fn view(&self) -> Option<&ViewSpec> {
        self.view.as_ref()
    }

    #[must_use]
    pub const fn collapsed(&self) -> bool {
        self.collapsed
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum PresentationOwner<A = PlanActorKey, N = PlanAnnotationKey> {
    Actor(A),
    Annotation(N),
}

impl<A, N> PresentationOwner<A, N> {
    pub fn map<B, M>(
        self,
        actor: impl FnOnce(A) -> B,
        annotation: impl FnOnce(N) -> M,
    ) -> PresentationOwner<B, M> {
        match self {
            Self::Actor(value) => PresentationOwner::Actor(actor(value)),
            Self::Annotation(value) => PresentationOwner::Annotation(annotation(value)),
        }
    }

    pub fn try_map<B, M, E>(
        self,
        actor: impl FnOnce(A) -> Result<B, E>,
        annotation: impl FnOnce(N) -> Result<M, E>,
    ) -> Result<PresentationOwner<B, M>, E> {
        Ok(match self {
            Self::Actor(value) => PresentationOwner::Actor(actor(value)?),
            Self::Annotation(value) => PresentationOwner::Annotation(annotation(value)?),
        })
    }
}

impl PresentationOwner {
    pub fn scope(&self) -> &[crate::scope_identity::ScopeSegment] {
        match self {
            Self::Actor(key) => &key.scope,
            Self::Annotation(key) => &key.scope,
        }
    }
    pub fn local(&self) -> &str {
        match self {
            Self::Actor(key) => key.local.as_str(),
            Self::Annotation(key) => &key.local,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SetPresentation {
    pub owner: PresentationOwner<AddressRef<PlanActorKey>, AddressRef<PlanAnnotationKey>>,
    pub presentation: Presentation<PlanActorKey>,
}

pub fn presentation_owner_from_value(
    value: Value,
    context: AddressContext,
) -> Result<
    PresentationOwner<AddressRef<PlanActorKey>, AddressRef<PlanAnnotationKey>>,
    PayloadRejection,
> {
    let mut fields = object_fields(value, "owner")?;
    if fields.len() != 1 {
        return Err(PayloadRejection::WrongCarrier { key: "owner" });
    }
    if let Some(value) = optional(&mut fields, "actor") {
        let address = decode_actor_address(value)?;
        if !context.admits(&address) {
            return Err(PayloadRejection::ArmNotAdmitted);
        }
        Ok(PresentationOwner::Actor(address))
    } else if let Some(value) = optional(&mut fields, "annotation") {
        let address = decode_annotation_address(value)?;
        if !context.admits(&address) {
            return Err(PayloadRejection::ArmNotAdmitted);
        }
        Ok(PresentationOwner::Annotation(address))
    } else {
        Err(PayloadRejection::WrongCarrier { key: "owner" })
    }
}

pub fn presentation_owner_value<A, N, E>(
    owner: &PresentationOwner<A, N>,
    actor: impl FnOnce(&A) -> Result<Value, E>,
    annotation: impl FnOnce(&N) -> Result<Value, E>,
) -> Result<Value, E> {
    let (key, value) = match owner {
        PresentationOwner::Actor(value) => ("actor", actor(value)?),
        PresentationOwner::Annotation(value) => ("annotation", annotation(value)?),
    };
    Ok(Value::object([(key, value)]).expect("one owner field"))
}

fn decode_view_spec(value: Value) -> Result<ViewSpec, PayloadRejection> {
    let mut fields = object_fields(value, "view")?;
    let config = take(&mut fields, "config")?;
    let kind = text_of(take(&mut fields, "kind")?, "kind")?;
    exhausted(fields)?;
    Ok(ViewSpec { kind, config })
}

fn decode_board_cell(value: Value) -> Result<BoardPlacement, PayloadRejection> {
    let mut fields = object_fields(value, "board")?;
    let col = bounded_u32(take(&mut fields, "col")?, "col")?;
    let h = bounded_u32(take(&mut fields, "h")?, "h")?;
    let row = bounded_u32(take(&mut fields, "row")?, "row")?;
    let w = bounded_u32(take(&mut fields, "w")?, "w")?;
    exhausted(fields)?;
    BoardPlacement::try_new(col, row, w, h)
        .map_err(|_| PayloadRejection::WrongCarrier { key: "board" })
}

fn bounded_u32(value: Value, key: &'static str) -> Result<u32, PayloadRejection> {
    u32::try_from(int_of(value, key)?).map_err(|_| PayloadRejection::WrongCarrier { key })
}

fn bounded_i32(value: Value, key: &'static str) -> Result<i32, PayloadRejection> {
    i32::try_from(int_of(value, key)?).map_err(|_| PayloadRejection::WrongCarrier { key })
}

fn decode_layout_point(value: Value) -> Result<LayoutPoint, PayloadRejection> {
    let mut fields = object_fields(value, "fixed")?;
    let x = bounded_i32(take(&mut fields, "x")?, "x")?;
    let y = bounded_i32(take(&mut fields, "y")?, "y")?;
    exhausted(fields)?;
    Ok(LayoutPoint {
        x: LayoutCoord::new(x),
        y: LayoutCoord::new(y),
    })
}

fn decode_layout_size(value: Value) -> Result<LayoutSize, PayloadRejection> {
    let mut fields = object_fields(value, "size")?;
    let h = unsigned_of(take(&mut fields, "h")?, "h")?;
    let w = unsigned_of(take(&mut fields, "w")?, "w")?;
    exhausted(fields)?;
    Ok(LayoutSize { w, h })
}

fn decode_relation(value: Value) -> Result<Relation, PayloadRejection> {
    let (tag, arguments) = decode_arm(value, "relation")?;
    if !arguments.is_empty() {
        return Err(PayloadRejection::WrongCarrier { key: "relation" });
    }
    Relation::from_tag(tag).ok_or(PayloadRejection::UnknownArm { tag })
}

fn decode_axis(value: Value) -> Result<Axis, PayloadRejection> {
    let (tag, arguments) = decode_arm(value, "axis")?;
    if !arguments.is_empty() {
        return Err(PayloadRejection::WrongCarrier { key: "axis" });
    }
    Axis::from_tag(tag).ok_or(PayloadRejection::UnknownArm { tag })
}

fn decode_anchor(value: Value) -> Result<Anchor<PlanActorKey>, PayloadRejection> {
    let (tag, mut arguments) = decode_arm(value, "anchor")?;
    if arguments.is_empty() {
        return match tag {
            1 => Ok(Anchor::Flow),
            other => Err(PayloadRejection::UnknownArm { tag: other }),
        };
    }
    if arguments.len() != 2 {
        return Err(PayloadRejection::WrongCarrier { key: "anchor" });
    }
    let argument = arguments.pop().expect("second argument");
    let target = decode_plan_actor_key(arguments.pop().expect("first argument"))?;
    match tag {
        2 => Ok(Anchor::Relative {
            target,
            relation: decode_relation(argument)?,
        }),
        3 => Ok(Anchor::Align {
            target,
            axis: decode_axis(argument)?,
        }),
        other => Err(PayloadRejection::UnknownArm { tag: other }),
    }
}

fn decode_presentation(value: Value) -> Result<Presentation<PlanActorKey>, PayloadRejection> {
    let mut fields = object_fields(value, "presentation")?;
    let anchor = optional(&mut fields, "anchor")
        .map(decode_anchor)
        .transpose()?;
    let board = optional(&mut fields, "board")
        .map(decode_board_cell)
        .transpose()?;
    let collapsed = bool_of(take(&mut fields, "collapsed")?, "collapsed")?;
    let fixed = optional(&mut fields, "fixed")
        .map(decode_layout_point)
        .transpose()?;
    let group = optional(&mut fields, "group")
        .map(|value| {
            let text = text_of(value, "group")?;
            GroupName::from_authored_syntax(text.as_bytes())
                .map_err(|_| PayloadRejection::NotCanonical { key: "group" })
        })
        .transpose()?;
    let label = optional(&mut fields, "label")
        .map(|value| text_of(value, "label"))
        .transpose()?;
    let size = optional(&mut fields, "size")
        .map(decode_layout_size)
        .transpose()?;
    let view = optional(&mut fields, "view")
        .map(decode_view_spec)
        .transpose()?;
    exhausted(fields)?;
    Ok(Presentation {
        label,
        group,
        anchor,
        fixed,
        size,
        board,
        view,
        collapsed,
    })
}

pub fn set_presentation_from_value(
    value: Value,
    context: AddressContext,
) -> Result<SetPresentation, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let owner = presentation_owner_from_value(take(&mut fields, "owner")?, context)?;
    let presentation = decode_presentation(take(&mut fields, "presentation")?)?;
    exhausted(fields)?;

    Ok(SetPresentation {
        owner,
        presentation,
    })
}

pub fn decode_set_presentation(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<SetPresentation, PayloadRejection> {
    set_presentation_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    use circular_core::{ObjectValue, encode};

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    #[test]
    fn annotation_owner_uses_retire_annotation_address_codec_and_refuses_old_owner() {
        let address = Value::array([Value::Int(1), key_object(&["cell"], "note")]);
        let annotation =
            super::super::annotation::decode_annotation_address(address.clone()).unwrap();
        let encoded = body(
            Value::object([("annotation", address.clone())]).unwrap(),
            presentation_value(vec![]),
        );
        let decoded =
            decode_set_presentation(&encoded, AddressContext::Mutation, CEILINGS).unwrap();
        assert_eq!(decoded.owner, PresentationOwner::Annotation(annotation));
        for owner in [
            address.clone(),
            Value::object([] as [(&str, Value); 0]).unwrap(),
            Value::object([("actor", address.clone()), ("annotation", address.clone())]).unwrap(),
            Value::object([("note", address)]).unwrap(),
        ] {
            assert_eq!(
                decode_set_presentation(
                    &body(owner, presentation_value(vec![])),
                    AddressContext::Mutation,
                    CEILINGS
                ),
                Err(PayloadRejection::WrongCarrier { key: "owner" })
            );
        }
    }

    fn plan_actor(scope: Vec<ScopeSegment>, local: &str) -> PlanActorKey {
        PlanActorKey {
            scope,
            local: crate::scope_identity::AuthoredLocal::try_new(local)
                .expect("test actor local is authored")
                .into(),
        }
    }

    fn key_object(scope: &[&str], local: &str) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("local".to_owned(), Value::String(local.to_owned())),
                (
                    "scope".to_owned(),
                    Value::Array(
                        scope
                            .iter()
                            .map(|name| {
                                Value::Array(vec![Value::Int(1), Value::String((*name).to_owned())])
                            })
                            .collect(),
                    ),
                ),
            ])
            .expect("two keys"),
        )
    }

    fn actor(scope: &[&str], local: &str) -> Value {
        Value::object([(
            "actor",
            Value::Array(vec![Value::Int(1), key_object(scope, local)]),
        )])
        .unwrap()
    }

    fn body(owner: Value, presentation: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([
            ("owner".to_owned(), owner),
            ("presentation".to_owned(), presentation),
        ])
        .expect("two keys");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    fn presentation_value(entries: Vec<(&str, Value)>) -> Value {
        let mut entries = entries;
        if !entries.iter().any(|(key, _)| *key == "collapsed") {
            entries.push(("collapsed", Value::Bool(false)));
        }
        axes(entries)
    }

    fn axes(entries: Vec<(&str, Value)>) -> Value {
        Value::Object(
            ObjectValue::try_from_entries(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value)),
            )
            .expect("keys do not overlap"),
        )
    }

    fn view(kind: &str, config: Value) -> Value {
        axes(vec![
            ("config", config),
            ("kind", Value::String(kind.to_owned())),
        ])
    }

    #[test]
    fn every_axis_opens() {
        let decoded = decode_set_presentation(
            &body(
                actor(&["gate", "inner"], "meter"),
                presentation_value(vec![
                    (
                        "anchor",
                        Value::Array(vec![
                            Value::Int(2),
                            key_object(&["gate", "inner"], "filter"),
                            Value::Int(1),
                        ]),
                    ),
                    (
                        "board",
                        axes(vec![
                            ("col", Value::Int(2)),
                            ("h", Value::Int(3)),
                            ("row", Value::Int(1)),
                            ("w", Value::Int(4)),
                        ]),
                    ),
                    ("collapsed", Value::Bool(true)),
                    (
                        "fixed",
                        axes(vec![("x", Value::Int(-120)), ("y", Value::Int(48))]),
                    ),
                    ("group", Value::String("telemetry".to_owned())),
                    ("label", Value::String("✓ tokens".to_owned())),
                    (
                        "size",
                        axes(vec![("h", Value::Int(90)), ("w", Value::Int(320))]),
                    ),
                    ("view", view("line-chart", Value::Null)),
                ]),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");

        let presentation = decoded.presentation;
        assert_eq!(presentation.label.as_deref(), Some("✓ tokens"));
        assert_eq!(
            presentation.group.as_ref().map(GroupName::as_str),
            Some("telemetry")
        );
        assert_eq!(
            presentation.fixed,
            Some(LayoutPoint::new(
                LayoutCoord::new(-120),
                LayoutCoord::new(48)
            ))
        );
        assert_eq!(presentation.size, Some(LayoutSize { w: 320, h: 90 }));
        assert!(presentation.collapsed, "collapsed is not an Option");
        assert_eq!(
            presentation.anchor,
            Some(Anchor::Relative {
                target: plan_actor(
                    vec![
                        ScopeSegment::Child("gate".to_owned()),
                        ScopeSegment::Child("inner".to_owned()),
                    ],
                    "filter",
                ),
                relation: Relation::Before,
            })
        );
        assert_eq!(
            presentation.board,
            Some(BoardPlacement::try_new(2, 1, 4, 3).expect("a board cell has no zero span"))
        );
        assert_eq!(
            presentation.view.expect("there is a view").kind,
            "line-chart"
        );
    }

    #[test]
    fn an_absent_axis_is_unset_and_not_a_rejection() {
        let decoded = decode_set_presentation(
            &body(
                actor(&["gate"], "meter"),
                presentation_value(vec![("label", Value::String("display".to_owned()))]),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("an update that carries only one axis is valid too");

        assert_eq!(decoded.presentation.label.as_deref(), Some("display"));
        assert_eq!(decoded.presentation.view, None);
        assert_eq!(decoded.presentation.fixed, None);
    }

    #[test]
    fn a_decomposed_group_name_arrives_composed() {
        let decoded = decode_set_presentation(
            &body(
                actor(&["gate"], "meter"),
                presentation_value(vec![("group", Value::String("cafe\u{0301}".to_owned()))]),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("a correct group name decodes");
        assert_eq!(
            decoded.presentation.group.as_ref().map(GroupName::as_str),
            Some("café")
        );
    }

    #[test]
    fn a_missing_collapsed_is_a_rejection_and_not_a_default() {
        assert_eq!(
            decode_set_presentation(
                &body(actor(&["gate"], "meter"), axes(Vec::new())),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::MissingKey("collapsed"))
        );
    }

    #[test]
    fn presentation_may_reference_a_synth_boundary_actor() {
        let synth = "_bno1_0123456789abcdef0123456789";
        let decoded = decode_set_presentation(
            &body(actor(&["gate"], synth), presentation_value(Vec::new())),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("presentation decodes");
        let PresentationOwner::Actor(AddressRef::Absolute(owner)) = decoded.owner else {
            panic!("test owner is absolute")
        };
        assert!(matches!(
            &owner.local,
            crate::scope_identity::ActorLocal::Synth(_)
        ));
        assert_eq!(owner.local.as_str(), synth);
    }

    #[test]
    fn an_empty_presentation_is_a_value() {
        let decoded = decode_set_presentation(
            &body(actor(&["gate"], "meter"), presentation_value(Vec::new())),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");
        assert_eq!(decoded.presentation, Presentation::default());
    }

    #[test]
    fn a_null_axis_is_rejected() {
        assert_eq!(
            decode_set_presentation(
                &body(
                    actor(&["gate"], "meter"),
                    presentation_value(vec![("label", Value::Null)])
                ),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::WrongCarrier { key: "label" })
        );
    }

    #[test]
    fn the_view_kind_is_stored_raw() {
        let spellings = ["  line-chart  ", "LINE-CHART", "line_chart"];
        let mut seen = std::collections::BTreeSet::new();
        for spelling in spellings {
            let decoded = decode_set_presentation(
                &body(
                    actor(&["gate"], "meter"),
                    presentation_value(vec![("view", view(spelling, Value::Null))]),
                ),
                AddressContext::Mutation,
                CEILINGS,
            )
            .expect("decodes");
            let kind = decoded.presentation.view.expect("there is a view").kind;
            assert_eq!(kind, spelling, "the spelling was trimmed");
            seen.insert(kind);
        }
        assert_eq!(seen.len(), 3, "three spellings folded into one");
    }

    #[test]
    fn the_view_config_stays_opaque() {
        let config = axes(vec![
            ("bins", Value::Int(24)),
            ("nested", Value::Array(vec![Value::Bool(true)])),
        ]);
        let decoded = decode_set_presentation(
            &body(
                actor(&["gate"], "meter"),
                presentation_value(vec![("view", view("histogram", config.clone()))]),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");
        assert_eq!(
            decoded.presentation.view.expect("there is a view").config,
            config
        );
    }

    #[test]
    fn a_coordinate_wider_than_the_layout_type_is_rejected() {
        let over = i64::from(i32::MAX) + 1;
        assert_eq!(
            decode_set_presentation(
                &body(
                    actor(&["gate"], "meter"),
                    presentation_value(vec![(
                        "fixed",
                        axes(vec![("x", Value::Int(over)), ("y", Value::Int(0))])
                    )])
                ),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::WrongCarrier { key: "x" })
        );
        assert!(
            decode_set_presentation(
                &body(
                    actor(&["gate"], "meter"),
                    presentation_value(vec![(
                        "fixed",
                        axes(vec![
                            ("x", Value::Int(i64::from(i32::MIN))),
                            ("y", Value::Int(i64::from(i32::MAX)))
                        ])
                    )])
                ),
                AddressContext::Mutation,
                CEILINGS
            )
            .is_ok(),
            "a boundary value is a value"
        );
    }

    #[test]
    fn a_half_fixed_point_is_rejected() {
        assert_eq!(
            decode_set_presentation(
                &body(
                    actor(&["gate"], "meter"),
                    presentation_value(vec![("fixed", axes(vec![("x", Value::Int(1))]))])
                ),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::MissingKey("y"))
        );
    }
}
