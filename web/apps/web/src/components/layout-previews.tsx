import type { Layout } from "@/lib/api";

/** Small illustrative mockups of the three Display layouts. Not screenshots. */
export function LayoutPreview({ layout }: { layout: Layout }) {
  return (
    <svg viewBox="0 0 160 100" role="img" aria-hidden className="h-auto w-full rounded-md">
      <rect width="160" height="100" fill="#11111b" />
      <rect width="160" height="12" fill="#181825" />
      <circle cx="150" cy="6" r="2" fill="#a6e3a1" />
      {layout === "cards" && (
        <>
          <Card x={6} color="#fab387" fill1={70} fill2={40} />
          <Card x={83} color="#89b4fa" fill1={45} fill2={12} />
        </>
      )}
      {layout === "gauges" && (
        <>
          <Ring cx={42} color="#fab387" value={0.7} text="70%" />
          <Ring cx={118} color="#89b4fa" value={0.45} text="45%" />
          <rect x="22" y="80" width="40" height="3" rx="1.5" fill="#313244" />
          <rect x="98" y="80" width="40" height="3" rx="1.5" fill="#313244" />
        </>
      )}
      {layout === "terminal" && (
        <g fontFamily="ui-monospace, Menlo, monospace" fontSize="8" fill="#a6e3a1">
          <text x="8" y="28">claude  5h 70%  resets 59m</text>
          <text x="8" y="40" fill="#cdd6f4">claude  7d 41%  resets 3d</text>
          <text x="8" y="58" fill="#a6e3a1">codex   5h 45%  resets 2h</text>
          <text x="8" y="70" fill="#f9e2af">codex   7d 12%  resets 5d</text>
          <rect x="8" y="78" width="5" height="9" fill="#cba6f7" />
        </g>
      )}
    </svg>
  );
}

function Card({ x, color, fill1, fill2 }: { x: number; color: string; fill1: number; fill2: number }) {
  return (
    <g>
      <rect x={x} y="18" width="71" height="76" rx="4" fill="#1e1e2e" />
      <rect x={x + 6} y="24" width="26" height="4" rx="2" fill={color} />
      <text x={x + 6} y="46" fontSize="14" fontWeight="700" fill={fill1 >= 50 ? "#a6e3a1" : "#f9e2af"} fontFamily="system-ui, sans-serif">
        {fill1}%
      </text>
      <rect x={x + 6} y="52" width="59" height="3" rx="1.5" fill="#313244" />
      <rect x={x + 6} y="52" width={(59 * fill1) / 100} height="3" rx="1.5" fill={fill1 >= 50 ? "#a6e3a1" : "#f9e2af"} />
      <text x={x + 6} y="72" fontSize="14" fontWeight="700" fill={fill2 >= 20 ? "#f9e2af" : "#f38ba8"} fontFamily="system-ui, sans-serif">
        {fill2}%
      </text>
      <rect x={x + 6} y="78" width="59" height="3" rx="1.5" fill="#313244" />
      <rect x={x + 6} y="78" width={(59 * fill2) / 100} height="3" rx="1.5" fill={fill2 >= 20 ? "#f9e2af" : "#f38ba8"} />
    </g>
  );
}

function Ring({ cx, color, value, text }: { cx: number; color: string; value: number; text: string }) {
  const r = 24;
  const length = Math.PI * r * 1.5; // 270-degree arc
  return (
    <g transform={`rotate(135 ${cx} 52)`}>
      <circle cx={cx} cy="52" r={r} fill="none" stroke="#313244" strokeWidth="6" strokeLinecap="round" strokeDasharray={`${length} 999`} />
      <circle cx={cx} cy="52" r={r} fill="none" stroke={color} strokeWidth="6" strokeLinecap="round" strokeDasharray={`${length * value} 999`} />
      <text x={cx} y="56" textAnchor="middle" fontSize="12" fontWeight="700" fill="#cdd6f4" transform={`rotate(-135 ${cx} 52)`} fontFamily="system-ui, sans-serif">
        {text}
      </text>
    </g>
  );
}
