import type { DefInfo } from "../api/generated/DefInfo";
import type { Persona } from "../api/generated/Persona";
import type { Product } from "../api/generated/Product";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";
import { ErrorLine } from "../ui/ErrorLine";
import { ListDetail } from "../ui/ListDetail";
import { ListRow } from "../ui/ListRow";
import { Section } from "../ui/Section";
import { usable } from "./definitions";
import { Internal } from "./Internal";
import { keyOptions, ready, replaces, SECTIONS, valueHint, valueOptions } from "./overrides";
import { ProductFields } from "./ProductFields";
import { canSave } from "./productForm";
import { dirty, hiddenToSend, overridesDirty, sameNames, type ProductsApi } from "./products";

// A port of crates/fleet-gui/src/views/management/products.rs: the products on the left, the selected one's form,
// its sections and Delete product on the right.

interface Props {
  /** The listed products; null until the list first answers. */
  products: Product[] | null;
  personas: Persona[];
  /** The definitions; null until they first load. */
  defs: DefInfo[] | null;
  tab: ProductsApi;
  onNewProduct: () => void;
  now: number;
}

export function Products({ products, personas, defs, tab, onNewProduct, now }: Props) {
  const { st } = tab;
  if (!products) {
    return (
      <div className="tabbed-page">
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
                <span className="list-row-name">{p.name}</span>
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
          product && <Detail product={product} personas={personas} defs={defs} tab={tab} now={now} />
        )
      }
    />
  );
}

interface DetailProps {
  product: Product;
  personas: Persona[];
  defs: DefInfo[] | null;
  tab: ProductsApi;
  now: number;
}

function Detail({ product, personas, defs, tab, now }: DetailProps) {
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
      <HiddenSpecialists defs={defs} tab={tab} />
      <Overrides defs={defs} tab={tab} />
      <Internal internal={tab.internal} now={now} />
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

/** egui's `hidden_section`: a chip per specialist that loads, then each hidden one that no longer does, struck through. */
function HiddenSpecialists({ defs, tab }: { defs: DefInfo[] | null; tab: ProductsApi }) {
  const { st } = tab;
  const names = defs && usable(defs, "specialist");
  const send = names && hiddenToSend(st, names);
  return (
    <Section title="Hidden specialists">
      <p className="note">
        Orchestrators of this product may use every specialist but the ones picked here. A task keeps the specialists it started with.
      </p>
      {names ? (
        <div className="chips">
          {names.map((name) => (
            <Chip key={name} selected={st.hidden.includes(name)} onClick={() => tab.toggleHidden(name)}>
              {name}
            </Chip>
          ))}
          {st.hidden
            .filter((n) => !names.includes(n))
            .map((name) => (
              <s key={name} className="stale-specialist" title="This specialist no longer loads: see Management">
                {name}
              </s>
            ))}
        </div>
      ) : (
        <p className="note">The specialists have not loaded.</p>
      )}
      <div>
        <Button disabled={!send || sameNames(send, st.savedHidden) || st.hiddenBusy} onClick={() => send && void tab.saveHidden(send)}>
          Save hidden specialists
        </Button>
      </div>
      {st.hiddenError !== null && <ErrorLine text={st.hiddenError} />}
    </Section>
  );
}

/** egui's `overrides_section`: a row per setting, then Add override and Save overrides, which sends the whole list. */
function Overrides({ defs, tab }: { defs: DefInfo[] | null; tab: ProductsApi }) {
  const { st, effective } = tab;
  const canSave = overridesDirty(st) && !st.overridesBusy && st.overrides.every(ready);
  return (
    <Section title="Overrides">
      <p className="note">Settings this product sets for itself. Every other setting follows the global one.</p>
      {st.overrides.map((row, i) => {
        const values = valueOptions(row.section, row.key, defs);
        const was = replaces(effective, row);
        return (
          <div key={i}>
            <div className="override-row">
              <Choice
                what="section"
                row={i}
                options={SECTIONS}
                value={row.section}
                onChange={(section) => tab.editOverride(i, { section })}
              />
              <Choice
                what="key"
                row={i}
                options={keyOptions(row.section, defs)}
                value={row.key}
                onChange={(key) => tab.editOverride(i, { key })}
              />
              {values ? (
                <Choice what="value" row={i} options={values} value={row.value} onChange={(value) => tab.editOverride(i, { value })} />
              ) : (
                <input
                  className="override-text"
                  aria-label={`Override value ${i + 1}`}
                  placeholder={valueHint(row.section)}
                  value={row.value}
                  onChange={(e) => tab.editOverride(i, { value: e.target.value })}
                  spellCheck={false}
                />
              )}
              {was !== null && <span className="override-replaces">{was}</span>}
              <Button className="ghost" aria-label={`Remove override ${i + 1}`} onClick={() => tab.removeOverride(i)}>
                Remove
              </Button>
            </div>
            {row.error !== null && <ErrorLine text={row.error} />}
          </div>
        );
      })}
      <div className="product-actions">
        <Button onClick={tab.addOverride}>Add override</Button>
        <Button variant="primary" disabled={!canSave} onClick={() => void tab.saveOverrides()}>
          Save overrides
        </Button>
      </div>
      {st.overridesError !== null && <ErrorLine text={st.overridesError} />}
    </Section>
  );
}

/** egui's `choice`: a dropdown named `Override {what} {row}`, showing `what` until picked. A value the options do not
 * list (a definition that is gone) is shown as it is. */
function Choice(props: { what: string; row: number; options: string[]; value: string; onChange: (v: string) => void }) {
  const { what, row, options, value, onChange } = props;
  const shown = value && !options.includes(value) ? [value, ...options] : options;
  return (
    <select
      className={`override-${what}`}
      aria-label={`Override ${what} ${row + 1}`}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    >
      <option value="" disabled>
        {what}
      </option>
      {shown.map((o) => (
        <option key={o} value={o}>
          {o}
        </option>
      ))}
    </select>
  );
}
