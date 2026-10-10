import { useEffect, useState } from "react";

export type PrdActivityUpdate = { message: string; at: number };

export function PrdActivity({ title, activity }: { title: string; activity: PrdActivityUpdate | null }) {
  const [startedAt] = useState(Date.now);
  const [now, setNow] = useState(startedAt);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const seconds = Math.floor((now - startedAt) / 1000);

  return (
    <div className="prd-activity">
      <div className="prd-activity__head">
        <span className="prd-doc__running"><span className="prd-doc__spinner" aria-hidden="true" />{title}</span>
        <span className="prd-activity__elapsed" aria-label={`${seconds} seconds elapsed`}>{Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}</span>
      </div>
      <p className="prd-activity__message" role="status">{activity?.message || "Preparing the workspace…"}</p>
      {now - (activity?.at ?? startedAt) >= 20000 && <p className="prd-doc__running-note">Waiting for the next agent update…</p>}
    </div>
  );
}
