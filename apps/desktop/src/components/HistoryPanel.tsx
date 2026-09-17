import type { TxnInfo } from "../types";

/** Transaction journal — every mutation, with actor + undo state. */
export default function HistoryPanel({ history }: { history: TxnInfo[] }) {
  return (
    <div className="panel historypanel">
      <h3>History</h3>
      <ul className="history">
        {history.map((t) => (
          <li key={t.id} className={t.undone ? "undone" : ""}>
            <div className="txnhead">
              <span className="idx">#{t.index}</span>
              <span className="actor">{t.actor}</span>
              <span className="dim small">{new Date(t.committed_at).toLocaleTimeString()}</span>
            </div>
            <div className="txnlabel">{t.label || "transaction"}</div>
            <div className="dim small">
              {t.commands.map((c) => c.type).join(", ")}
              {t.undone && " — undone"}
            </div>
          </li>
        ))}
        {history.length === 0 && <li className="dim">No transactions yet.</li>}
      </ul>
    </div>
  );
}
