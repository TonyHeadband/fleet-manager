import { useEffect, useId, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import { Button } from "../ui/Button";
import { ProductFields } from "./ProductFields";
import { canSave, emptyForm, newProduct, repoUpserts } from "./productForm";

// A port of the New product modal in crates/fleet-gui/src/app/products.rs (product_form_modal, save_new_product).

interface Props {
  client: FleetClient;
  personas: Persona[];
  /** Made: it becomes the active product. */
  onCreated: (p: Product) => void;
  onClose: () => void;
  /** egui's toasts: a repo the new product could not add. */
  say: (text: string) => void;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function NewProduct({ client, personas, onCreated, onClose, say }: Props) {
  const titleId = useId();
  const [form, setForm] = useState(emptyForm);
  const [error, setError] = useState<string | null>(null);
  // A second Save while the first is out is ignored, as egui's `busy`.
  const busy = useRef(false);

  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [onClose]);

  const save = async () => {
    if (busy.current || !canSave(form)) return;
    busy.current = true;
    setError(null);
    let made: Product;
    try {
      made = await client.createProduct(newProduct(form));
    } catch (e) {
      busy.current = false;
      setError(message(e));
      return;
    }
    // The product is made; a repo it could not add is said, and the rest still go.
    for (const repo of repoUpserts(form)) {
      await client
        .upsertRepo(made.id, repo)
        .catch((e: unknown) => say(`Created ${made.name}, but could not add the repo ${repo.name}: ${message(e)}`));
    }
    onCreated(made);
  };

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog new-product" role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <h2 id={titleId}>New product</h2>
        <ProductFields form={form} onChange={setForm} personas={personas} repoInfo={() => undefined} repoErrors={{}} />
        <div className="form-actions end">
          <Button variant="primary" disabled={!canSave(form)} onClick={() => void save()}>
            Save
          </Button>
          <Button onClick={onClose}>Cancel</Button>
        </div>
        {error !== null && <p className="danger">{error}</p>}
      </div>
    </div>
  );
}
