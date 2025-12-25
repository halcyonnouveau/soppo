import { json } from "@sveltejs/kit";
import type { RequestHandler } from "./$types";
import { spawn } from "child_process";
import { writeFile, readFile, rm, mkdir } from "fs/promises";
import { tmpdir } from "os";
import { join } from "path";
import { randomUUID } from "crypto";

interface CompileRequest {
  source: string;
}

interface CompileResponse {
  goCode?: string;
  output?: string;
  compileError?: string;
  runError?: string;
}

async function runSopCompiler(
  source: string,
): Promise<{ code?: string; error?: string }> {
  const tempDir = join(tmpdir(), `soppo-${randomUUID()}`);
  const inputFile = join(tempDir, "main.sop");
  const outputFile = join(tempDir, "main.go");
  const goModFile = join(tempDir, "go.mod");

  try {
    await mkdir(tempDir, { recursive: true });
    await writeFile(inputFile, source);
    await writeFile(
      goModFile,
      `module playground

go 1.25

require github.com/halcyonnouveau/soppo/runtime v0.1.0

replace github.com/halcyonnouveau/soppo/runtime => /go/src/github.com/halcyonnouveau/soppo/runtime
`,
    );

    return new Promise((resolve) => {
      // sop build <file> outputs to <file>.go in same directory
      // Run from tempDir so go.mod is found
      const proc = spawn("sop", ["build", "main.sop"], { cwd: tempDir });
      let stderr = "";

      proc.stderr.on("data", (data) => {
        stderr += data.toString();
      });

      proc.on("close", async (exitCode) => {
        if (exitCode !== 0) {
          resolve({ error: stderr || "Compilation failed" });
        } else {
          try {
            const goCode = await readFile(outputFile, "utf-8");
            resolve({ code: goCode });
          } catch {
            resolve({ error: "Failed to read output file" });
          }
        }

        // Cleanup
        try {
          await rm(tempDir, { recursive: true, force: true });
        } catch {
          // Ignore cleanup errors
        }
      });
    });
  } catch (err) {
    return { error: `Failed to write temp file: ${err}` };
  }
}

async function runGoPlayground(
  goCode: string,
): Promise<{ output?: string; error?: string }> {
  try {
    const response = await fetch("https://go.dev/_/compile", {
      method: "POST",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded",
      },
      body: new URLSearchParams({
        version: "2",
        body: goCode,
        withVet: "true",
      }),
    });

    if (!response.ok) {
      return { error: `Go Playground returned ${response.status}` };
    }

    const result = await response.json();

    if (result.Errors) {
      return { error: result.Errors };
    }

    // Handle events format (stdout/stderr messages)
    let output = "";
    if (result.Events) {
      for (const event of result.Events) {
        if (event.Kind === "stdout" || event.Kind === "stderr") {
          output += event.Message;
        }
      }
    }

    return { output: output || "(no output)" };
  } catch (err) {
    return { error: `Failed to call Go Playground: ${err}` };
  }
}

export const POST: RequestHandler = async ({ request }) => {
  const body: CompileRequest = await request.json();

  if (!body.source) {
    return json(
      { compileError: "No source provided" } satisfies CompileResponse,
      { status: 400 },
    );
  }

  // Step 1: Compile Soppo to Go
  const compileResult = await runSopCompiler(body.source);

  if (compileResult.error) {
    return json({
      compileError: compileResult.error,
    } satisfies CompileResponse);
  }

  const goCode = compileResult.code!;

  // Step 2: Run on Go Playground
  const runResult = await runGoPlayground(goCode);

  const response: CompileResponse = {
    goCode,
  };

  if (runResult.error) {
    response.runError = runResult.error;
  } else {
    response.output = runResult.output;
  }

  return json(response);
};
