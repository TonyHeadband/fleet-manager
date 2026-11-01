import type { ReactElement } from "react";
import { ColumnBars } from "./ColumnBars";
import { Donut } from "./Donut";
import { Meter } from "./Meter";
import { RankedBars } from "./RankedBars";
import { SAMPLES } from "./samples";
import { SegmentedBar } from "./SegmentedBar";
import { Stat } from "./Stat";
import { StatusTimeline } from "./StatusTimeline";
import { TimeSeries } from "./TimeSeries";
import "./charts.css";

export interface CatalogEntry {
  id: string;
  name: string;
  purpose: string;
  /** Minimum size in the Supervisor grid's cells. */
  minW: number;
  minH: number;
  preview(): ReactElement;
}

export const CATALOG: CatalogEntry[] = [
  { id: "stat", name: "Stat", purpose: "A big number with its change and a sparkline.", minW: 2, minH: 1, preview: () => <Stat {...SAMPLES.agentsLive} /> },
  { id: "column-bars", name: "Column bars", purpose: "One column per category, side by side.", minW: 2, minH: 2, preview: () => <ColumnBars {...SAMPLES.agentsByStatus} /> },
  { id: "segmented-bar", name: "Segmented bar", purpose: "Parts of a whole on a scale.", minW: 3, minH: 1, preview: () => <SegmentedBar {...SAMPLES.storageByKind} /> },
  { id: "donut", name: "Donut", purpose: "Shares of a total.", minW: 2, minH: 2, preview: () => <Donut {...SAMPLES.costByProduct} /> },
  { id: "time-series", name: "Time series", purpose: "Lines over time on one axis, with a crosshair.", minW: 4, minH: 2, preview: () => <TimeSeries {...SAMPLES.tokensPerHour} /> },
  { id: "ranked-bars", name: "Ranked bars", purpose: "A sorted list of label, bar and value.", minW: 2, minH: 2, preview: () => <RankedBars {...SAMPLES.topTasksByCost} /> },
  { id: "meter", name: "Meter", purpose: "A value against its limit, with threshold marks.", minW: 2, minH: 1, preview: () => <Meter {...SAMPLES.dailyBudget} /> },
  { id: "status-timeline", name: "Status timeline", purpose: "The state each entity was in over a time range.", minW: 4, minH: 2, preview: () => <StatusTimeline {...SAMPLES.agentActivity} /> },
];

/** The widget library's shape: every block with what it is for, its minimum size and a live preview. */
export function ChartCatalog() {
  return (
    <div className="chart-catalog">
      {CATALOG.map((e) => (
        <section key={e.id} className="chart-catalog-entry outlined" aria-label={e.name}>
          <h2 className="chart-catalog-name">
            {e.name}
            <span className="chart-catalog-min">min {e.minW}×{e.minH}</span>
          </h2>
          <p className="chart-catalog-purpose">{e.purpose}</p>
          {/* Blocks fill their container: a grid cell is half the drawer's width, so the minimum size gives a definite one. */}
          <div className="chart-catalog-preview" style={{ width: `calc(${e.minW} * var(--layout-drawer) / 2)`, minHeight: `calc(${e.minH} * var(--layout-drawer) / 4)` }}>
            {e.preview()}
          </div>
        </section>
      ))}
    </div>
  );
}
