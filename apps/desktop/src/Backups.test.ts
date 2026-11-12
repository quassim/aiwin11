import { describe, expect, it } from "vitest";
import { describeBackup } from "./Backups";

describe("backup names", () => {
  it("shows date, time and reason, and leaves unknown names as they are", () => {
    expect(describeBackup("aiwin11-20261113-142530123-manual.sqlite3")).toBe("13/11/2026 14:25 UTC, manuelle");
    expect(describeBackup("aiwin11-20261113-142530123-before-migration.sqlite3")).toContain("avant mise à jour");
    expect(describeBackup("autre.sqlite3")).toBe("autre.sqlite3");
  });
});
