/** Renders key=value pairs with colored keys for easy scanning. */
export default function KvPairs({ pairs, vertical }: { pairs: Array<{ key: string; value: string }>; vertical?: boolean }) {
  if (pairs.length === 0) return <span className="kv-empty">(no fields)</span>
  if (vertical) {
    return (
      <div className="kv-pairs-vertical">
        {pairs.map((p, i) => (
          <div key={i} className="kv-pair">
            <span className="kv-key">{p.key}</span>
            <span className="kv-eq">=</span>
            <span className="kv-value">{p.value}</span>
          </div>
        ))}
      </div>
    )
  }
  return (
    <span className="kv-pairs">
      {pairs.map((p, i) => (
        <span key={i} className="kv-pair">
          <span className="kv-key">{p.key}</span>
          <span className="kv-eq">=</span>
          <span className="kv-value">{p.value}</span>
        </span>
      ))}
    </span>
  )
}
