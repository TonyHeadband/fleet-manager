import "./charts.css";

/** What a screen reader gets in place of the marks: every label with its formatted value. */
export function HiddenTable({ caption, rows }: { caption: string; rows: [string, string][] }) {
  return (
    <table className="chart-hidden">
      <caption>{caption}</caption>
      <tbody>
        {rows.map(([label, value], i) => (
          <tr key={i}>
            <th scope="row">{label}</th>
            <td>{value}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
