// <PokerbotTable />: a reusable, self-contained React table for one human vs
// Pokerbot session. It renders only what the engine's JSON says: the legal
// actions, chip amounts, cards and results all come from PublicState. It
// contains no poker rules.

import { type CSSProperties, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { type ClientOptions, createPokerbotClient, type PokerbotClient } from './client';
import { type PokerbotTheme, resolveTheme, type ThemeProp } from './theme';
import { phrase, wordLog } from './wording';
import {
  type BotKind,
  type CardCode,
  type DepthBB,
  type LegalActionDTO,
  PokerbotError,
  type PublicState,
  type StreetName,
} from './types';

export interface PokerbotTableProps {
  /** URL of the `.pokerbot` policy artifact (required for `bot="cfr"`). */
  policyUrl?: string;
  /** Or the artifact bytes directly. */
  policy?: Uint8Array;
  /** Stack depth in big blinds; the policy must be trained for it. */
  depthBB?: DepthBB;
  /** Session seed (BigInt or non-negative safe integer). Same seed, same cards. */
  seed?: bigint | number;
  /** `'cfr'` (default) or the explicitly untrained `'baseline'`. */
  bot?: BotKind;
  /** `'sepia'` (default), `'terminal'`, or palette overrides. */
  theme?: ThemeProp;
  /** Engine placement; the default runs it in a Web Worker. */
  worker?: ClientOptions['worker'];
  /** Called with every new public state (e.g. for analytics). */
  onState?: (s: PublicState) => void;
  className?: string;
  style?: CSSProperties;
}

const SUITS: Record<string, { sym: string; red: boolean; name: string }> = {
  c: { sym: '♣', red: false, name: 'clubs' },
  d: { sym: '♦', red: true, name: 'diamonds' },
  h: { sym: '♥', red: true, name: 'hearts' },
  s: { sym: '♠', red: false, name: 'spades' },
};

function rankText(r: string): string {
  return r === 'T' ? '10' : r;
}

function Card({ code }: { code: CardCode }) {
  const rank = code.slice(0, 1);
  const suit = SUITS[code.slice(1, 2)] ?? { sym: '?', red: false, name: '?' };
  return (
    <span
      className={`pokerbot-card${suit.red ? ' pokerbot-red' : ''}`}
      data-card={code}
      role="img"
      aria-label={`${rankText(rank)} of ${suit.name}`}
    >
      <span className="pokerbot-rank">{rankText(rank)}</span>
      <span className="pokerbot-suit">{suit.sym}</span>
    </span>
  );
}

function CardBack() {
  return <span className="pokerbot-card pokerbot-back" role="img" aria-label="face-down card" />;
}

function Slot() {
  return <span className="pokerbot-card pokerbot-slot" aria-hidden="true" />;
}

function who(w: 'human' | 'pokerbot'): string {
  return w === 'human' ? 'You' : 'Pokerbot';
}

/** Button text in the CLI menu's wording, e.g. "Raise to 6 (pot)". */
function actionLabel(a: LegalActionDTO, s: PublicState): { main: string; sub: string } {
  const cap = (t: string) => t.charAt(0).toUpperCase() + t.slice(1);
  const add = a.chipsToCommit > 0 ? `+${a.chipsToCommit}` : '';
  if (a.type === 'all_in') return { main: `All-in to ${a.raiseTo}`, sub: add };
  if (a.type === 'call' && a.chipsToCommit === s.myStack) return { main: `Call ${a.chipsToCommit} (all-in)`, sub: '' };
  const toCall = Math.max(0, s.botStreetChips - s.myStreetChips);
  const main = cap(phrase(a.type, a.raiseTo, a.chipsToCommit, s.myStreetChips, toCall, true));
  return { main, sub: a.type === 'half_pot' || a.type === 'pot' ? add : '' };
}

const STREETS: StreetName[] = ['preflop', 'flop', 'turn', 'river'];

function signed(n: number): string {
  return n > 0 ? `+${n}` : `${n}`;
}

function themeVars(t: PokerbotTheme): CSSProperties {
  const v: Record<string, string> = {};
  for (const [k, val] of Object.entries(t)) v[`--pokerbot-${k}`] = val;
  return v as CSSProperties;
}

function randomSeed(): bigint {
  const a = new Uint32Array(2);
  crypto.getRandomValues(a);
  return (BigInt(a[0] ?? 0) << 32n) | BigInt(a[1] ?? 0);
}

export function PokerbotTable(props: PokerbotTableProps) {
  const { policyUrl, policy, depthBB = 20, bot: botProp = 'cfr', worker, onState } = props;
  const theme = useMemo(() => resolveTheme(props.theme), [props.theme]);
  const [initialSeed] = useState(() => props.seed ?? randomSeed());
  const seed = props.seed ?? initialSeed;
  // The user may explicitly switch to the baseline if the policy cannot load.
  const [botChoice, setBotChoice] = useState<BotKind>(botProp);
  useEffect(() => setBotChoice(botProp), [botProp]);

  const clientRef = useRef<PokerbotClient | null>(null);
  const [state, setState] = useState<PublicState | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<PokerbotError | null>(null);
  const [loadError, setLoadError] = useState<PokerbotError | null>(null);
  const [mode, setMode] = useState<PokerbotClient['mode'] | null>(null);
  const logRef = useRef<HTMLElement | null>(null);
  // Keep the newest log entry in view (the log scrolls on small screens).
  const logLen = state?.actionLog.length ?? 0;
  const handNo = state?.handNumber ?? 0;
  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logLen, handNo]);
  const onStateRef = useRef(onState);
  onStateRef.current = onState;

  const show = useCallback((s: PublicState) => {
    setState(s);
    onStateRef.current?.(s);
  }, []);

  useEffect(() => {
    const client = createPokerbotClient({ worker });
    clientRef.current = client;
    let live = true;
    setBusy(true);
    setState(null);
    setError(null);
    setLoadError(null);
    client
      .start({
        depthBB,
        seed,
        bot: botChoice,
        ...(botChoice === 'cfr' ? (policy ? { policy } : policyUrl ? { policyUrl } : {}) : {}),
      })
      .then((s) => {
        if (!live) return;
        setMode(client.mode);
        show(s);
      })
      .catch((e: unknown) => {
        if (live) setLoadError(e instanceof PokerbotError ? e : new PokerbotError(String(e)));
      })
      .finally(() => {
        if (live) setBusy(false);
      });
    return () => {
      live = false;
      client.dispose();
      if (clientRef.current === client) clientRef.current = null;
    };
  }, [depthBB, seed, botChoice, policy, policyUrl, worker, show]);

  const run = useCallback(
    (f: (c: PokerbotClient) => Promise<PublicState>) => {
      const c = clientRef.current;
      if (!c) return;
      setBusy(true);
      setError(null);
      f(c)
        .then(show)
        .catch((e: unknown) => setError(e instanceof PokerbotError ? e : new PokerbotError(String(e))))
        .finally(() => setBusy(false));
    },
    [show],
  );

  const rootClass = `pokerbot-table${props.className ? ` ${props.className}` : ''}`;
  const rootStyle = { ...themeVars(theme), ...props.style };

  if (loadError) {
    return (
      <div className={rootClass} style={rootStyle} data-testid="pokerbot-table" data-status="error">
        <style>{CSS}</style>
        <div className="pokerbot-panel pokerbot-load-error" role="alert">
          <strong>Could not start the CFR bot.</strong>
          <p data-testid="load-error">{loadError.message}</p>
          {botChoice === 'cfr' && (
            <>
              <p className="pokerbot-dim">
                Train a policy with <code>scripts/train_smoke.sh</code> and serve the <code>.pokerbot</code> file, or
                play the untrained baseline instead. The baseline is never substituted automatically.
              </p>
              <button type="button" className="pokerbot-btn" data-testid="use-baseline" onClick={() => setBotChoice('baseline')}>
                Play the untrained baseline
              </button>
            </>
          )}
        </div>
      </div>
    );
  }

  if (!state) {
    return (
      <div className={rootClass} style={rootStyle} data-testid="pokerbot-table" data-status="loading">
        <style>{CSS}</style>
        <div className="pokerbot-panel pokerbot-dim" aria-live="polite">
          Loading engine and policy…
        </div>
      </div>
    );
  }

  const s = state;
  const over = s.terminal !== null;
  const myTurn = s.actor === 'human' && !over;
  const revealed = s.botCards;
  const lines = wordLog(s.actionLog, s);
  const logByStreet = STREETS.map((st) => ({ st, items: lines.filter((l) => l.street === st) })).filter(
    (g) => g.items.length > 0,
  );
  const board: Array<CardCode | null> = [0, 1, 2, 3, 4].map((i) => s.board[i] ?? null);

  return (
    <div
      className={rootClass}
      style={rootStyle}
      data-testid="pokerbot-table"
      data-status={over ? 'hand-over' : myTurn ? 'your-turn' : 'waiting'}
      data-street={s.street}
      data-hand={s.handNumber}
      aria-busy={busy}
    >
      <style>{CSS}</style>
      <header className="pokerbot-header">
        <div className="pokerbot-title">
          POKERBOT <span className="pokerbot-dim">heads-up no-limit hold'em</span>
        </div>
        <div className="pokerbot-meta">
          <span>Hand {s.handNumber}</span>
          <span>
            {s.depthBB}BB · blinds {s.smallBlind}/{s.bigBlind}
          </span>
          <span data-testid="score">
            Session {signed(s.session.score.human)} ({s.session.handsCompleted} played)
          </span>
        </div>
      </header>

      <section className="pokerbot-panel pokerbot-seat pokerbot-bot" aria-label="Pokerbot">
        <div className="pokerbot-seat-head">
          <span className="pokerbot-name">
            Pokerbot{s.button === 'pokerbot' && <span className="pokerbot-button" title="dealer button">D</span>}
          </span>
          <span className="pokerbot-stack" data-testid="bot-stack">
            {s.botStack} chips
          </span>
        </div>
        <div className="pokerbot-cards" data-testid="bot-cards">
          {revealed ? (
            revealed.map((c) => <Card key={c} code={c} />)
          ) : (
            <>
              <CardBack />
              <CardBack />
            </>
          )}
        </div>
        <div className="pokerbot-dim pokerbot-street-chips">{s.botStreetChips > 0 ? `in: ${s.botStreetChips}` : ' '}</div>
      </section>

      <section className="pokerbot-panel pokerbot-board" aria-label="Board">
        <div className="pokerbot-cards" data-testid="board">
          {board.map((c, i) => (c ? <Card key={c} code={c} /> : <Slot key={`slot${i}`} />))}
        </div>
        <div className="pokerbot-pot" data-testid="pot">
          {over ? `Hand over · ${s.street}` : `Pot ${s.potChips} · ${s.street}`}
        </div>
      </section>

      <section className="pokerbot-panel pokerbot-seat pokerbot-me" aria-label="You">
        <div className="pokerbot-seat-head">
          <span className="pokerbot-name">
            You{s.button === 'human' && <span className="pokerbot-button" title="dealer button">D</span>}
          </span>
          <span className="pokerbot-stack" data-testid="my-stack">
            {s.myStack} chips
          </span>
        </div>
        <div className="pokerbot-cards" data-testid="my-cards">
          {s.myCards.map((c) => (
            <Card key={c} code={c} />
          ))}
        </div>
        <div className="pokerbot-dim pokerbot-street-chips">{s.myStreetChips > 0 ? `in: ${s.myStreetChips}` : ' '}</div>
      </section>

      {over ? (
        <section className="pokerbot-panel pokerbot-result" data-testid="result" aria-live="polite">
          <ResultText s={s} />
          <button
            type="button"
            className="pokerbot-btn pokerbot-primary"
            data-testid="next-hand"
            disabled={busy}
            onClick={() => run((c) => c.nextHand())}
          >
            Next hand
          </button>
        </section>
      ) : (
        <section className="pokerbot-actions" aria-label="Your actions" data-testid="actions">
          {myTurn ? (
            s.legalActions.map((a) => {
              const l = actionLabel(a, s);
              return (
                <button
                  type="button"
                  key={a.type}
                  className={`pokerbot-btn${a.type === 'fold' ? ' pokerbot-fold' : ''}`}
                  data-action={a.type}
                  disabled={busy}
                  onClick={() => run((c) => c.step(a.type))}
                >
                  <span>{l.main}</span>
                  {l.sub && <small>{l.sub}</small>}
                </button>
              );
            })
          ) : (
            <span className="pokerbot-dim">Pokerbot is thinking…</span>
          )}
        </section>
      )}

      {error && (
        <div className="pokerbot-error" role="alert" data-testid="error">
          {error.message}
        </div>
      )}

      <section className="pokerbot-panel pokerbot-log" aria-label="Hand log" data-testid="log" ref={logRef}>
        {logByStreet.length === 0 && <div className="pokerbot-dim">Blinds posted.</div>}
        {logByStreet.map((g) => (
          <div key={g.st}>
            <div className="pokerbot-log-street">{g.st}</div>
            <ol>
              {g.items.map((e, i) => (
                <li key={i} className={e.actor === 'human' ? 'pokerbot-log-me' : undefined}>
                  {e.text}
                </li>
              ))}
            </ol>
          </div>
        ))}
      </section>

      <footer className="pokerbot-footer">
        <div>
          <strong>Opponent:</strong> <span data-testid="bot-label">{s.bot.label}</span>
          {s.bot.artifactChecksum && <span className="pokerbot-dim"> · policy {s.bot.artifactChecksum}</span>}
          {mode && <span className="pokerbot-dim"> · engine: {mode}</span>}
        </div>
        <p data-testid="disclaimer">{s.bot.disclaimer}</p>
        <p className="pokerbot-dim">{s.note}</p>
      </footer>
    </div>
  );
}

function ResultText({ s }: { s: PublicState }) {
  const me = s.payoff?.human ?? 0;
  const t = s.terminal;
  const line =
    me > 0 ? `You win ${me} chips` : me < 0 ? `Pokerbot wins ${-me} chips` : 'Split pot';
  return (
    <div>
      <div className={`pokerbot-result-line ${me > 0 ? 'pokerbot-win' : me < 0 ? 'pokerbot-lose' : ''}`}>
        {line}
        <span className="pokerbot-dim">
          {' '}
          ({t?.reason === 'fold' ? `${t.winner === 'human' ? 'Pokerbot' : 'you'} folded` : 'showdown'})
        </span>
      </div>
      {s.showdown && (
        <div className="pokerbot-showdown" data-testid="showdown">
          <div>You: {s.showdown.myHand}</div>
          <div>Pokerbot: {s.showdown.botHand}</div>
        </div>
      )}
    </div>
  );
}

const CSS = `
.pokerbot-table{box-sizing:border-box;width:100%;max-width:760px;margin:0 auto;padding:12px;
  background:var(--pokerbot-bg);color:var(--pokerbot-ink);font-family:var(--pokerbot-font);font-size:15px;line-height:1.4;
  border:2px solid var(--pokerbot-frame);border-radius:10px;display:grid;gap:10px;
  grid-template-columns:1fr 1fr;grid-template-areas:"head head" "bot board" "me board" "act act" "err err" "log log" "foot foot";
  overflow-wrap:anywhere}
.pokerbot-table *,.pokerbot-table *::before,.pokerbot-table *::after{box-sizing:border-box}
.pokerbot-header{grid-area:head;display:flex;flex-wrap:wrap;justify-content:space-between;gap:4px 12px;
  border-bottom:1px dashed var(--pokerbot-frame);padding-bottom:6px}
.pokerbot-title{font-weight:700;letter-spacing:.12em;color:var(--pokerbot-accent)}
.pokerbot-title .pokerbot-dim{letter-spacing:0;font-weight:400}
.pokerbot-meta{display:flex;flex-wrap:wrap;gap:4px 12px;font-size:13px}
.pokerbot-panel{background:var(--pokerbot-panel);border:1px solid var(--pokerbot-frame);border-radius:8px;padding:8px 10px;min-width:0}
.pokerbot-bot{grid-area:bot}.pokerbot-me{grid-area:me}
.pokerbot-board{grid-area:board;display:flex;flex-direction:column;justify-content:center;align-items:center;gap:8px}
.pokerbot-seat-head{display:flex;justify-content:space-between;gap:8px;margin-bottom:6px}
.pokerbot-name{font-weight:700}
.pokerbot-button{display:inline-block;margin-left:6px;padding:0 5px;border-radius:50%;font-size:11px;
  background:var(--pokerbot-accent);color:var(--pokerbot-bg)}
.pokerbot-cards{display:flex;flex-wrap:wrap;gap:6px;justify-content:center;min-height:3.4em}
.pokerbot-card{display:inline-flex;flex-direction:column;align-items:center;justify-content:center;
  width:2.6em;height:3.4em;border-radius:6px;background:var(--pokerbot-cardFace);color:var(--pokerbot-cardBlack);
  border:1px solid var(--pokerbot-frame);font-weight:700;line-height:1.05;box-shadow:1px 2px 0 rgba(0,0,0,.18)}
.pokerbot-card.pokerbot-red{color:var(--pokerbot-cardRed)}
.pokerbot-suit{font-size:1.15em}
.pokerbot-back{background:repeating-linear-gradient(45deg,var(--pokerbot-cardBack) 0 5px,var(--pokerbot-cardBackInk) 5px 7px)}
.pokerbot-slot{background:transparent;border-style:dashed;box-shadow:none;opacity:.55}
.pokerbot-pot{font-weight:700;color:var(--pokerbot-accent)}
.pokerbot-street-chips{font-size:13px;text-align:center;min-height:1.2em;margin-top:4px}
.pokerbot-actions,.pokerbot-result{grid-area:act}
.pokerbot-actions{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,7.5em),1fr));gap:8px;min-height:3em;align-items:center}
.pokerbot-result{display:flex;flex-wrap:wrap;gap:10px;justify-content:space-between;align-items:center}
.pokerbot-result-line{font-weight:700}
.pokerbot-win{color:var(--pokerbot-win)}.pokerbot-lose{color:var(--pokerbot-lose)}
.pokerbot-showdown{font-size:13px}
.pokerbot-btn{font:inherit;cursor:pointer;min-height:44px;padding:6px 10px;border-radius:8px;
  border:2px solid var(--pokerbot-frame);background:var(--pokerbot-cardFace);color:var(--pokerbot-ink);
  display:flex;flex-direction:column;align-items:center;justify-content:center;gap:0}
.pokerbot-btn small{font-size:11px;opacity:.75}
.pokerbot-btn:hover:not(:disabled){border-color:var(--pokerbot-accent)}
.pokerbot-btn:focus-visible{outline:3px solid var(--pokerbot-accent);outline-offset:2px}
.pokerbot-btn:disabled{opacity:.5;cursor:progress}
.pokerbot-primary{background:var(--pokerbot-accent);color:var(--pokerbot-bg);border-color:var(--pokerbot-accent);font-weight:700}
.pokerbot-fold{color:var(--pokerbot-lose)}
.pokerbot-error{grid-area:err;color:var(--pokerbot-warn);font-size:13px}
.pokerbot-log{grid-area:log;font-size:13px;max-height:12em;overflow-y:auto}
.pokerbot-log ol{margin:0 0 4px;padding-left:1.4em}
.pokerbot-log-street{text-transform:uppercase;letter-spacing:.1em;color:var(--pokerbot-dim);font-size:11px}
.pokerbot-log-me{color:var(--pokerbot-accent)}
.pokerbot-footer{grid-area:foot;font-size:12px;border-top:1px dashed var(--pokerbot-frame);padding-top:6px}
.pokerbot-footer p{margin:4px 0}
.pokerbot-dim{color:var(--pokerbot-dim)}
.pokerbot-load-error{grid-column:1/-1}
.pokerbot-load-error code{font-family:inherit}
@media (max-width:560px){
  .pokerbot-table{grid-template-columns:1fr;grid-template-areas:"head" "bot" "board" "me" "act" "err" "log" "foot";padding:8px;font-size:14px}
  .pokerbot-card{width:2.3em;height:3.1em}
}
`;
