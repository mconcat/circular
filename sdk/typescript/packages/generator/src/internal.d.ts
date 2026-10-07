import type { CompactedDeclarationCommandList, ScopeId } from '@circular/protocol';
import type { ProgramGeneratorOptions, generateProgram } from './generator-entry.js';

/** The constructor spelling of a standing actor: projectInput / projectOutput for a child scope's input / output, else its actor type. */
export declare function actorSpelling(actorType: string, scope: readonly unknown[]): string;
export declare function bindingIdentifier(name: string): boolean;
export declare function replicatorPolicy(config: unknown): boolean;
/** A replicator's Template value has exactly one boundary inlet: its config's `in` names one topic. */
export declare function replicatorInlet(config: unknown): boolean;
export declare function compactTemplateCommands(commands: readonly unknown[], relative?: boolean): CompactedDeclarationCommandList;
export declare const EXPORT_MARK_NAMES: ReadonlySet<string>;
export declare const FIXED_SOURCE_RECONSTRUCTION_PORTS: Readonly<Record<string, { readonly in_ports: readonly { readonly id: string; readonly primary: boolean }[]; readonly out_ports: readonly { readonly id: string; readonly primary: boolean }[] }>>;
export declare function generateProgramFromSession(
  session: { authoringSnapshot(scope: ScopeId, pageLimit: number): Promise<unknown> },
  options: ProgramGeneratorOptions & { readonly scope?: ScopeId; readonly pageLimit?: number },
): Promise<ReturnType<typeof generateProgram>>;
