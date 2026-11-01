import "./charts.css";
import { paintVar, type Paint } from "./paint";

export function Legend({ items }: { items: { label: string; paint: Paint }[] }) {
  return (
    <ul className="chart-legend">
      {items.map((it, i) => (
        <li key={i}>
          <span className="chart-swatch" style={{ background: paintVar(it.paint) }} />
          {it.label}
        </li>
      ))}
    </ul>
  );
}
