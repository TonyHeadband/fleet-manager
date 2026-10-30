import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import { Button } from "../ui/Button";
import { ErrorLine } from "../ui/ErrorLine";
import { ListDetail } from "../ui/ListDetail";
import { ListRow } from "../ui/ListRow";
import { ProductFields } from "./ProductFields";
import { canSave } from "./productForm";
import { dirty, type ProductsApi } from "./products";

// A port of crates/fleet-gui/src/views/management/products.rs: the products on the left, the selected one's form,
// its sections and Delete product on the right.

interface Props {
  /** The listed products; null until the list first answers. */
  products: Product[] | null;
  personas: Persona[];
  tab: ProductsApi;
  onNewProduct: () => void;
}

export function Products({ products, personas, tab, onNewProduct }: Props) {
  const { st } = tab;
  if (!products) {
    return (
      <div className="management-page">
        <p className="note">Loading…</p>
      </div>
    );
  }
  const product = products.find((p) => p.id === st.selected && p.id === st.forProduct) ?? null;
  return (
    <ListDetail
      width="management"
      listLabel="Products"
      detailLabel={product?.name ?? "Product"}
      list={
        <ul>
          {products.map((p) => (
            <li key={p.id}>
              <ListRow name={p.name} selected={p.id === st.selected} onSelect={() => tab.select(p.id)}>
                <span className="product-name">{p.name}</span>
              </ListRow>
            </li>
          ))}
        </ul>
      }
      footer={<Button onClick={onNewProduct}>+ New product</Button>}
      detail={
        products.length === 0 ? (
          <p className="note">No product yet. Create one with New product.</p>
        ) : (
          // Until the form is synced to the selection, there is nothing of it to draw.
          product && <Detail product={product} personas={personas} tab={tab} />
        )
      }
    />
  );
}

function Detail({ product, personas, tab }: { product: Product; personas: Persona[]; tab: ProductsApi }) {
  const { st } = tab;
  const changed = dirty(st);
  return (
    <div className="product-pane">
      <h2>{product.name}</h2>
      <ProductFields
        form={st.form}
        onChange={tab.setForm}
        personas={personas}
        repoInfo={(name) => product.repos.find((r) => r.name === name)}
        repoErrors={st.repoErrors}
      />
      <div>
        <div className="product-actions">
          <Button variant="primary" disabled={!(canSave(st.form) && changed && !st.busy)} onClick={() => void tab.save()}>
            Save
          </Button>
          <Button disabled={!(changed && !st.busy)} onClick={tab.revert}>
            Revert
          </Button>
        </div>
        {st.error !== null && <ErrorLine text={st.error} />}
      </div>
      <div>
        {st.confirmDelete ? (
          <>
            <p className="product-confirm">Delete {product.name}? This cannot be undone.</p>
            <div className="product-actions">
              <Button variant="quiet-danger" onClick={() => void tab.confirmDelete(product.id)}>
                Confirm delete
              </Button>
              <Button onClick={tab.cancelDelete}>Cancel</Button>
            </div>
          </>
        ) : (
          <Button variant="quiet-danger" onClick={tab.askDelete}>
            Delete product
          </Button>
        )}
        {st.deleteError !== null && <ErrorLine text={st.deleteError} />}
      </div>
    </div>
  );
}
