import { useEffect, useState } from "react";
import type { FleetClient } from "../api/client";
import { isKnown } from "../api/forms";
import type { Product } from "../api/generated/Product";
import type { UiSchema } from "../api/generated/UiSchema";
import { Form } from "../ui/Form";

interface Props {
  client: FleetClient;
  schema: UiSchema;
  today: string;
}

/** The daemon's forms, live, for comparing with the egui app. Not their final place in the UI. */
export function FormsPage({ client, schema, today }: Props) {
  const [products, setProducts] = useState<Product[] | null>(null);
  const [product, setProduct] = useState("");
  const [done, setDone] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    client.products().then(
      (l) => {
        setProducts(l.products);
        setProduct((p) => p || (l.products[0]?.id ?? ""));
      },
      (e: unknown) => setLoadError(e instanceof Error ? e.message : String(e)),
    );
  }, [client]);

  const create = schema.forms.find((f) => f.id === "release_create");
  if (loadError) return <p role="alert">{loadError}</p>;
  if (!products) return <p className="loading">Loading…</p>;
  return (
    <section className="forms-page">
      <label>
        Product
        <select value={product} onChange={(e) => setProduct(e.target.value)}>
          {products.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      {!create || !isKnown(create) ? (
        <p>This build cannot draw the release form the daemon describes.</p>
      ) : (
        <Form
          key={product}
          form={create}
          today={today}
          onSubmit={async (values) => {
            setDone(null);
            await client.submitForm(create, { product }, values);
            setDone(`Created ${values.version?.trim()}`);
          }}
        />
      )}
      {done && <p className="done">{done}</p>}
    </section>
  );
}
