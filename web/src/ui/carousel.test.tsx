import { act, fireEvent, render } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { motion } from "../generated/layout";
import { END_SLACK } from "../shell/entry";
import { Carousel } from "./Carousel";

// The shared carousel: panes side by side on a track that slides to the current one. The panel's tabs keep every
// pane once visited; a view's tabs keep only the pane they leave, until the slide ends.

const ids = ["a", "b", "c"] as const;
const draw = (id: string) => <p>{`pane ${id}`}</p>;
const track = () => document.querySelector<HTMLElement>(".carousel > .pane-track")!;
const slots = () => [...track().querySelectorAll<HTMLElement>(":scope > .pane-slot")];
const texts = () => slots().map((s) => s.textContent);

afterEach(() => {
  delete document.documentElement.dataset.animations;
  vi.useRealTimers();
});

describe("Carousel", () => {
  it("slides_the_track_to_the_current_pane", () => {
    const { rerender } = render(<Carousel ids={ids} current="a" keep="visited" render={draw} />);
    expect(track().style.getPropertyValue("--index")).toBe("0");
    rerender(<Carousel ids={ids} current="c" keep="visited" render={draw} />);
    expect(track().style.getPropertyValue("--index")).toBe("2");
    expect(slots()[0]).toHaveAttribute("inert");
    expect(slots()[0]).toHaveAttribute("aria-hidden", "true");
    expect(slots()[2]).not.toHaveAttribute("inert");
  });

  it("keeps_every_visited_pane", () => {
    const { rerender } = render(<Carousel ids={ids} current="a" keep="visited" render={draw} />);
    expect(texts()).toEqual(["pane a", "", ""]);
    rerender(<Carousel ids={ids} current="b" keep="visited" render={draw} />);
    fireEvent.transitionEnd(track());
    expect(texts()).toEqual(["pane a", "pane b", ""]);
  });

  it("a_sliding_view_keeps_only_a_still_copy_of_the_pane_it_leaves_until_the_slide_ends", () => {
    const mounted: string[] = [];
    const Live = ({ id }: { id: string }) => {
      useEffect(() => {
        mounted.push(id);
        return () => void mounted.splice(mounted.indexOf(id), 1);
      }, [id]);
      return <p>{`pane ${id}`}</p>;
    };
    const live = (id: string) => <Live id={id} />;
    const { rerender } = render(<Carousel ids={ids} current="a" keep="sliding" render={live} />);
    rerender(<Carousel ids={ids} current="c" keep="sliding" render={live} />);
    // It still shows as it slides away, but nothing of it runs: the page left stops its work at once.
    expect(texts()).toEqual(["pane a", "", "pane c"]);
    expect(mounted).toEqual(["c"]);
    fireEvent.transitionEnd(track());
    expect(texts()).toEqual(["", "", "pane c"]);
  });

  it("going_back_mid_slide_shows_the_live_pane_alone", () => {
    const { rerender } = render(<Carousel ids={ids} current="a" keep="sliding" render={draw} />);
    rerender(<Carousel ids={ids} current="b" keep="sliding" render={draw} />);
    rerender(<Carousel ids={ids} current="a" keep="sliding" render={draw} />);
    expect(slots()[0]!.querySelectorAll("p")).toHaveLength(1);
  });

  it("a_lost_slide_end_still_drops_the_old_pane", () => {
    vi.useFakeTimers();
    const { rerender } = render(<Carousel ids={ids} current="a" keep="sliding" render={draw} />);
    rerender(<Carousel ids={ids} current="b" keep="sliding" render={draw} />);
    act(() => vi.advanceTimersByTime(motion.slide + END_SLACK + 1));
    expect(texts()).toEqual(["", "pane b", ""]);
  });

  it("with_motion_off_the_old_pane_goes_at_once", () => {
    document.documentElement.dataset.animations = "off";
    const { rerender } = render(<Carousel ids={ids} current="a" keep="sliding" render={draw} />);
    rerender(<Carousel ids={ids} current="b" keep="sliding" render={draw} />);
    expect(texts()).toEqual(["", "pane b", ""]);
  });
});
