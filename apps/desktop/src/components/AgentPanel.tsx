import { useState } from "react";
import { api } from "../api";
import type { AgentReport } from "../types";

/** Agent panel: natural-language goal → real commands in one transaction. */
export default function AgentPanel({ onDone }: { onDone: () => void }) {
  const [goal, setGoal] = useState("");
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<AgentReport | null>(null);
  const [err, setErr] = useState("");

  const run = async () => {
    if (!goal.trim() || busy) return;
    setBusy(true);
    setErr("");
    try {
      const r = await api.agentRun(goal);
      setReport(r);
      onDone();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="agentbar">
      <span className="agentlabel">Agent</span>
      <input
        placeholder='e.g. "create another cube next to sdk-cube named housing"'
        value={goal}
        onChange={(e) => setGoal(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && void run()}
        disabled={busy}
      />
      <button className="primary" onClick={() => void run()} disabled={busy}>
        {busy ? "running…" : "Run"}
      </button>
      {report && (
        <span className={`agentresult ${report.status}`} title={report.summary}>
          {report.status}: {report.steps.map((s) => s.command).join(" → ")}
        </span>
      )}
      {err && <span className="error">{err}</span>}
    </div>
  );
}
