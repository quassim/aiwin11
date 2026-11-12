import { describe, expect, it } from "vitest";
import { type LoadedModel, placementAdvice, placementLabel } from "./placement";

const model = (placement: LoadedModel["placement"], gpu_percent: number | null = null): LoadedModel => ({
  name: "m",
  size: 100,
  size_vram: 60,
  placement,
  gpu_percent,
});

describe("model placement", () => {
  it("labels each placement", () => {
    expect(placementLabel(model("gpu", 100))).toBe("Entièrement sur le GPU");
    expect(placementLabel(model("partial", 60))).toContain("60 % sur le GPU");
    expect(placementLabel(model("cpu", 0))).toContain("processeur seulement");
    expect(placementLabel(model("unknown"))).toBe("Répartition GPU inconnue");
  });

  it("gives advice only when a model is not fully on the GPU, CPU fallback first", () => {
    expect(placementAdvice([model("gpu", 100)])).toBeUndefined();
    expect(placementAdvice([model("gpu", 100), model("partial", 60)])).toContain("VRAM libre");
    expect(placementAdvice([model("partial", 60), model("cpu", 0)])).toContain("pilote");
  });
});
