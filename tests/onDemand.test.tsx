import { describe, it, expect } from "bun:test";
import { render } from "@testing-library/react";
import { OnDemand } from "../src/components/OnDemand";

describe("OnDemand", () => {
  it("renders nothing until it is wanted", () => {
    const { container } = render(
      <OnDemand when={false}>
        <p>hello</p>
      </OnDemand>,
    );
    expect(container.textContent).toBe("");
  });

  it("mounts the first time it is wanted and stays mounted after, so closing can animate", () => {
    const view = render(
      <OnDemand when={false}>
        <p>hello</p>
      </OnDemand>,
    );
    view.rerender(
      <OnDemand when>
        <p>hello</p>
      </OnDemand>,
    );
    expect(view.container.textContent).toBe("hello");
    view.rerender(
      <OnDemand when={false}>
        <p>hello</p>
      </OnDemand>,
    );
    expect(view.container.textContent).toBe("hello");
  });

  it("mounts at once when it starts out wanted", () => {
    const { container } = render(
      <OnDemand when>
        <p>now</p>
      </OnDemand>,
    );
    expect(container.textContent).toBe("now");
  });
});
