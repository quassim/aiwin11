export type Placement = "gpu" | "partial" | "cpu" | "unknown";
export type LoadedModel = {
  name: string;
  size: number;
  size_vram: number;
  placement: Placement;
  gpu_percent: number | null;
};

export function placementLabel(model: LoadedModel): string {
  switch (model.placement) {
    case "gpu":
      return "Entièrement sur le GPU";
    case "partial":
      return `${model.gpu_percent ?? 0} % sur le GPU, le reste sur le processeur (plus lent)`;
    case "cpu":
      return "Sur le processeur seulement (aucune mémoire GPU utilisée)";
    default:
      return "Répartition GPU inconnue";
  }
}

/** Advice when at least one loaded model is not fully on the GPU, otherwise nothing. */
export function placementAdvice(models: LoadedModel[]): string | undefined {
  if (models.some((model) => model.placement === "cpu")) {
    return "Un modèle tourne sur le processeur : vérifie le pilote de la carte graphique, ou la VRAM est entièrement occupée par d’autres modèles ou applications.";
  }
  if (models.some((model) => model.placement === "partial")) {
    return "Un modèle ne tient pas entièrement dans la VRAM libre : choisis un modèle plus petit ou plus quantifié, ou décharge les autres modèles.";
  }
  return undefined;
}
