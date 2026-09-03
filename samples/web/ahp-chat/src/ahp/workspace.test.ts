import { describe, expect, it } from "vitest"

import { formatWorkspace, workspaceInputToFileUri } from "./workspace"

describe("workspaceInputToFileUri", () => {
  it("converts absolute Windows paths to canonical file URIs", () => {
    expect(workspaceInputToFileUri("C:\\Users\\Ada\\work space")).toBe("file:///C:/Users/Ada/work%20space")
  })

  it("converts absolute POSIX paths to canonical file URIs", () => {
    expect(workspaceInputToFileUri("/home/ada/work space")).toBe("file:///home/ada/work%20space")
    expect(formatWorkspace("file:///home/ada/work%20space")).toBe("/home/ada/work space")
  })

  it("converts and formats UNC paths without using the browser platform", () => {
    expect(workspaceInputToFileUri("\\\\fileserver\\team share\\project")).toBe("file://fileserver/team%20share/project")
    expect(formatWorkspace("file://fileserver/team%20share/project")).toBe("\\\\fileserver\\team share\\project")
  })

  it("accepts an existing file URI without checking the filesystem", () => {
    expect(workspaceInputToFileUri("file:///C:/Users/Ada/work%20space")).toBe("file:///C:/Users/Ada/work%20space")
    expect(formatWorkspace("file:///C:/Users/Ada/work%20space")).toBe("C:\\Users\\Ada\\work space")
  })

  it("rejects non-absolute or non-path file URI forms", () => {
    expect(() => workspaceInputToFileUri("file:relative")).toThrow("absolute path")
    expect(() => workspaceInputToFileUri("file:///work?query")).toThrow("query or fragment")
    expect(() => workspaceInputToFileUri("file:///work#fragment")).toThrow("query or fragment")
  })
})
