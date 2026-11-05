@SETLOCAL
@IF NOT DEFINED NODE_PATH (
  @SET "NODE_PATH=\pnpm\store\node_modules"
) ELSE (
  @SET "NODE_PATH=\pnpm\store\node_modules;%NODE_PATH%"
)
@IF EXIST "%~dp0\node.exe" (
  "%~dp0\node.exe"  "%~dp0\..\@openai\codex\bin\codex.js" %*
) ELSE (
  @SET PATHEXT=%PATHEXT:;.JS;=;%
  node  "%~dp0\..\@openai\codex\bin\codex.js" %*
)
