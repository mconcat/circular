/** @packageDocumentation Typed observation descriptors without a client-owned observation-name enumeration. */

import type {
  CircularValue,
  DeliveryDiscipline,
  Generation,
  Incarnation,
  NonNegativeInteger,
  QueryDescriptor,
  Stamp,
  SubscriptionDescriptor,
  TargetName,
} from "@circular/protocol";

/** The two and only two observation value families. */
export type ObservationFamily = "Occurrence" | "Fact";

/** A registered observation kind together with its canonical first-order arguments. */
export interface ObservationKindKey<Args = CircularValue> {
  /** The registered flat observation target name. */
  readonly name: TargetName;
  /** Canonical arguments interpreted only by the owning observation registration. */
  readonly args: Args;
}

/** An immutable typed occurrence emitted at a logical stamp by one runtime incarnation. */
export interface Occurrence<Kind> {
  /** The occurrence-family discriminant. */
  readonly family: "Occurrence";
  /** Stamp inherited from the fact that caused this occurrence. */
  readonly at: Stamp;
  /** Runtime incarnation that emitted the occurrence. */
  readonly by: Incarnation;
  /** Registration-owned typed occurrence payload. */
  readonly kind: Kind;
}

/** A complete conflation key for a registered fact domain. */
export interface FactKey<Subject> {
  /** The registered flat fact-domain name. */
  readonly name: TargetName;
  /** The registration-owned complete fact subject. */
  readonly subject: Subject;
}

/** A typed fact transition that replaces, rather than appends to, current fact state. */
export interface Fact<Subject, Value> {
  /** The fact-family discriminant. */
  readonly family: "Fact";
  /** Complete key used as the subscription conflation slot. */
  readonly key: FactKey<Subject>;
  /** Registration-owned current fact value. */
  readonly value: Value;
  /** Incarnation or registry generation that owns this transition lineage. */
  readonly generation: Generation;
  /** Monotonic sequence within that one key and generation lineage. */
  readonly sequence: NonNegativeInteger;
}

/** The complete observation value union used by generic tooling. */
export type ObservationValue<OccurrenceKind, FactSubject, FactValue> =
  | Occurrence<OccurrenceKind>
  | Fact<FactSubject, FactValue>;

/** An occurrence target descriptor whose loss behavior is derived from registered density. */
export type OccurrenceSubscriptionDescriptor<
  Args,
  Kind,
  Anchor,
  Discipline extends Extract<DeliveryDiscipline, "lossless" | "credit">,
> = SubscriptionDescriptor<Args, Occurrence<Kind>, Anchor, Discipline>;

/** A fact target descriptor, always conflated by its complete fact key. */
export type FactSubscriptionDescriptor<Args, Subject, Value, Anchor> = SubscriptionDescriptor<
  Args,
  Fact<Subject, Value>,
  Anchor,
  "conflated"
>;

/** A feature-owned observation scan descriptor; the client does not enumerate scan kinds. */
export type ObservationQueryDescriptor<Args, Item, Anchor, Paging extends "none" | "cursor"> = QueryDescriptor<
  Args,
  Item,
  Anchor,
  Paging
>;

/** A registration row projected by an observation feature owner for generic GUI tooling. */
export interface ObservationDescriptor<Args, Payload, Anchor, Discipline extends DeliveryDiscipline> {
  /** Whether payload values are immutable occurrences or replacing facts. */
  readonly family: ObservationFamily;
  /** The feature-owned subscription descriptor containing the actual flat target name. */
  readonly subscription: SubscriptionDescriptor<Args, Payload, Anchor, Discipline>;
}
