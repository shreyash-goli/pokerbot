// Typed DTOs for the JSON produced by crates/mako-wasm (see
// docs/WEB_INTEGRATION.md). These types only describe data; every game
// rule (legality, chip amounts, showdown) is decided by the Rust engine.

export type MakoAction = 'fold' | 'check' | 'call' | 'half_pot' | 'pot' | 'all_in';
export type DepthBB = 20 | 50 | 100;
export type BotKind = 'cfr' | 'baseline';
export type Who = 'human' | 'mako';
export type StreetName = 'preflop' | 'flop' | 'turn' | 'river' | 'showdown';

/** Two-character card code: rank `23456789TJQKA` + suit `cdhs`, e.g. `"Ah"`. */
export type CardCode = string;

export interface LegalActionDTO {
  type: MakoAction;
  chipsToCommit: number;
  raiseTo: number;
}

export interface ActionLogEntry {
  actor: Who;
  street: StreetName;
  type: MakoAction;
  raiseTo: number;
  chipsCommitted: number;
}

export type Terminal = null | { reason: 'fold'; winner: Who } | { reason: 'showdown' };

export interface BotStats {
  decisions: number;
  policyHits: number;
  fallbacks: number;
}

export interface BotSummary {
  kind: BotKind;
  label: string;
  artifactChecksum: string | null;
  notGto: true;
  disclaimer: string;
  statsAtLastHandEnd: BotStats;
}

export interface PublicState {
  schema: 'mako.public_state.v1';
  handNumber: number;
  seed: string;
  depthBB: DepthBB;
  startingStack: number;
  smallBlind: number;
  bigBlind: number;
  button: Who;
  street: StreetName;
  board: CardCode[];
  potChips: number;
  actor: Who | null;
  myCards: [CardCode, CardCode];
  myStack: number;
  botStack: number;
  myStreetChips: number;
  botStreetChips: number;
  legalActions: LegalActionDTO[];
  actionLog: ActionLogEntry[];
  terminal: Terminal;
  payoff: null | { human: number; mako: number };
  /** Present ONLY when `terminal.reason === 'showdown'`. */
  botCards?: [CardCode, CardCode];
  /** Present ONLY at showdown. */
  showdown?: { myHand: string | null; botHand: string | null };
  session: { handsCompleted: number; score: { human: number; mako: number }; poisoned: boolean };
  bot: BotSummary;
  note: string;
}

export type HistoryEvent =
  | {
      type: 'hand_started';
      button: Who;
      smallBlind: number;
      bigBlind: number;
      startingStacks: Record<Who, number>;
      blindsPosted: Record<Who, number>;
    }
  | { type: 'hole_cards'; seat: 'human'; cards: [CardCode, CardCode] }
  | {
      type: 'action';
      actor: Who;
      street: StreetName;
      action: MakoAction;
      raiseTo: number;
      chipsCommitted: number;
    }
  | { type: 'board_dealt'; street: StreetName; cards: CardCode[] }
  | {
      type: 'showdown';
      hands: Array<{ seat: Who; holeCards: [CardCode, CardCode]; bestFive: CardCode[]; hand: string }>;
    }
  | {
      type: 'hand_ended';
      terminal: Exclude<Terminal, null>;
      payoff: Record<Who, number>;
      finalStacks: Record<Who, number>;
    };

export interface HistoryDTO {
  schema: 'mako.history.v1';
  events: HistoryEvent[];
}

export interface StrategyInfo {
  schema: 'mako.strategy_info.v1';
  kind: BotKind;
  label: string;
  notGto: true;
  disclaimer: string;
  trainedInBrowser: false;
  policy: null | {
    checksum: string;
    label: string;
    trainerId: string;
    trainerSeed: string;
    iterations: number;
    infosets: number;
    depthBB: number;
    smallBlind: number;
    bigBlind: number;
    startingStack: number;
    abstractionFingerprint: string;
    abstraction: {
      version: number;
      flopBuckets: number;
      turnBuckets: number;
      riverBuckets: number;
      flopSamples: number;
      turnSamples: number;
    };
  };
  statsAtLastHandEnd: BotStats;
}

/** Stable error codes thrown by the engine (message prefix). */
export type MakoErrorCode =
  | 'invalid_depth'
  | 'invalid_bot'
  | 'invalid_seed'
  | 'policy_missing'
  | 'policy_invalid'
  | 'policy_rules_mismatch'
  | 'unknown_action'
  | 'illegal_action'
  | 'hand_over'
  | 'hand_in_progress'
  | 'not_your_turn'
  | 'bot_error'
  | 'engine_error'
  | 'session_poisoned'
  | 'policy_fetch_failed'
  | 'client_disposed'
  | 'not_started'
  | 'worker_failed';

export class MakoError extends Error {
  readonly code: MakoErrorCode | string;
  constructor(message: string) {
    super(message);
    this.name = 'MakoError';
    const i = message.indexOf(':');
    this.code = i > 0 ? message.slice(0, i) : 'engine_error';
  }
}
