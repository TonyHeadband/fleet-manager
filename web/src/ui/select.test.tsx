import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Modal } from "./Modal";
import { Select } from "./Select";

// A form's single choice in the app's own selector (FilterMenu), not the browser's: a combobox that opens a listbox
// of options, labelled by its field's caption.

const options = [
  { value: "", label: "Solo" },
  { value: "lead", label: "lead" },
];

describe("Select", () => {
  it("shows_its_pick_and_is_named_by_its_caption", () => {
    render(
      <>
        <label htmlFor="type">Type</label>
        <Select id="type" value="" options={options} onChange={() => {}} />
      </>,
    );
    const combo = screen.getByRole("combobox", { name: "Type" });
    expect(combo).toHaveTextContent("Solo");
    expect(combo).toHaveAttribute("aria-haspopup", "listbox");
    expect(combo).toHaveClass("filter-button");
  });

  it("picks_from_a_listbox_and_closes", async () => {
    const onChange = vi.fn();
    render(<Select label="Type" value="" options={options} onChange={onChange} />);
    await userEvent.click(screen.getByRole("combobox", { name: "Type" }));
    const list = screen.getByRole("listbox");
    expect(within(list).getByRole("option", { name: "Solo" })).toHaveAttribute("aria-selected", "true");
    await userEvent.click(within(list).getByRole("option", { name: "lead" }));
    expect(onChange).toHaveBeenCalledWith("lead");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("a_value_with_no_option_shows_its_placeholder_or_itself", () => {
    const { rerender } = render(<Select label="Persona" value="gone" placeholder="Pick a persona" options={options} onChange={() => {}} />);
    expect(screen.getByRole("combobox", { name: "Persona" })).toHaveTextContent("Pick a persona");
    rerender(<Select label="Checkout" value="/raw/path" options={options} onChange={() => {}} />);
    expect(screen.getByRole("combobox", { name: "Checkout" })).toHaveTextContent("/raw/path");
  });

  it("repicking_the_same_option_changes_nothing", async () => {
    const onChange = vi.fn();
    render(<Select label="Type" value="lead" options={options} onChange={onChange} />);
    await userEvent.click(screen.getByRole("combobox", { name: "Type" }));
    await userEvent.click(screen.getByRole("option", { name: "lead" }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("escape_in_a_dialog_closes_only_the_menu", async () => {
    const onClose = vi.fn();
    render(
      <Modal label="New task" onClose={onClose}>
        <Select label="Type" value="" options={options} onChange={() => {}} />
      </Modal>,
    );
    await userEvent.click(screen.getByRole("combobox", { name: "Type" }));
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });
});
