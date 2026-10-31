import { useEffect, useState } from "react";
import type { FleetClient } from "../api/client";
import { isKnown } from "../api/forms";
import type { AgentConfig } from "../api/generated/AgentConfig";
import type { Product } from "../api/generated/Product";
import type { UiSchema } from "../api/generated/UiSchema";
import { Form } from "../ui/Form";
import { Select } from "../ui/Select";

interface Props {
  client: FleetClient;
  schema: UiSchema;
  today: string;
}

/** The daemon's forms, live, for comparing with the egui app. Not their final place in the UI. */
export function FormsPage({ client, schema, today }: Props) {
  const [products, setProducts] = useState<Product[] | null>(null);
  const [config, setConfig] = useState<AgentConfig | null>(null);
  const [product, setProduct] = useState("");
  const [done, setDone] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([client.products(), client.agentConfig()]).then(
      ([l, c]) => {
        setProducts(l.products);
        setProduct((p) => p || (l.products[0]?.id ?? ""));
        setConfig(c);
      },
      (e: unknown) => setLoadError(e instanceof Error ? e.message : String(e)),
    );
  }, [client]);

  const create = schema.forms.find((f) => f.id === "release_create");
  const instructions = schema.forms.find((f) => f.id === "instructions");
  if (loadError) return <p role="alert">{loadError}</p>;
  if (!products || !config) return <p className="loading">Loading…</p>;
  return (
    <section className="forms-page">
      <label>
        Product
        <Select label="Product" value={product} options={products.map((p) => ({ value: p.id, label: p.name }))} onChange={setProduct} />
      </label>
      {products.length === 0 ? (
        <p>There are no products to add a release to.</p>
      ) : !create || !isKnown(create) ? (
        <p>This build cannot draw the release form the daemon describes.</p>
      ) : (
        <Form
          key={product}
          form={create}
          today={today}
          onSubmit={async (submit, values) => {
            setDone(null);
            await client.submitForm(create, submit, { product }, values);
            setDone(`Created ${values.version?.trim()}`);
          }}
        />
      )}
      {instructions && isKnown(instructions) && (
        <Form
          form={instructions}
          today={today}
          initial={{ instructions: config.instructions }}
          onSubmit={async (submit, values) => {
            setDone(null);
            await client.submitForm(instructions, submit, {}, values);
            setDone("Saved");
          }}
        />
      )}
      {done && <p className="done">{done}</p>}
    </section>
  );
}
