import { describe, expect, it } from "vitest"

import { formatWorkspace, workspaceInputToFileUri } from "./workspace"

describe("workspaceInputToFileUri", () => {
  it("converts absolute Windows paths to canonical file URIs", () => {
    expect(workspaceInputToFileUri("C:\\Users\\Ada\\work space")).toBe("file:///C:/Users/Ada/work%20space")
  })

  it("converts absolute POSIX paths to canonical file URIs", () => {
    expect(workspaceInputToFileUri("/home/ada/work space")).toBe("file:///home/ada/work%20space")
  })

  it("accepts an existing file URI without checking the filesystem", () => {
    expect(workspaceInputToFileUri("file:///C:/Users/Ada/work%20space")).toBe("file:///C:/Users/Ada/work%20space")
    expect(formatWorkspace("file:///C:/Users/Ada/work%20space")).toBe("C:\\Users\\Ada\\work space")
  })
})
