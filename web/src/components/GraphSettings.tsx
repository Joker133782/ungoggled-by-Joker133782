import { GRAPH_GROUPS, useGraphGroups } from "../graph-groups";
import { PanelHead } from "./PanelHead";
import { IconChart } from "./icons";

/** Settings → Graphs: which groups System → Last 30 minutes shows. */
export function GraphSettings() {
  const [groups, setGroup] = useGraphGroups();
  return (
    <section className="panel graph-settings" id="graph-settings">
      <PanelHead icon={<IconChart />} title="Graphs" wiki="graphs" />
      <ul className="toggle-list">
        {GRAPH_GROUPS.map((g) => (
          <li key={g.id}>
            <label className="toggle">
              <input
                type="checkbox"
                checked={groups[g.id]}
                onChange={(e) => setGroup(g.id, e.currentTarget.checked)}
              />
              {g.title}
            </label>
            <span className="help">{g.detail}</span>
          </li>
        ))}
      </ul>
      <p className="status-line">
        Which graphs <a href="#system/graphs">System → Last 30 minutes</a>{" "}
        shows. Saved on this device only.
      </p>
    </section>
  );
}
