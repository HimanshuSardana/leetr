--- leetr.nvim — talk to a running Leetr instance from nvim.
---
--- Install (either):
---   * run `leetr install-nvim-plugin`, or
---   * copy this file to ~/.config/nvim/lua/leetr.lua and add
---       require("leetr").setup()
---     to your init.lua.
---
--- Commands:
---   :LeetrStatus        show what Leetr currently has open
---   :LeetrRun           run the sample tests
---   :LeetrSubmit        submit (Leetr refuses unless the problem is solved)
---   :LeetrResult        show the last judge verdict
---   :LeetrOpen <slug>   ask Leetr to open a problem
---   :LeetrPull          load Leetr's current buffer into this window
---   :LeetrPush          send this buffer to Leetr
---   :LeetrPort <n>      point this session at a different Leetr port
---
--- With `setup({ auto_push = true })` (the default) saving a file that lives in
--- the Leetr workspace pushes it to the app automatically, so there is no
--- copy-paste and no manual sync.

local M = {}

M.port = tonumber(vim.env.LEETR_PORT) or 9527

local notify = function(message, level)
  vim.notify("leetr: " .. message, level or vim.log.levels.INFO)
end

local function endpoint(path)
  return ("http://127.0.0.1:%d%s"):format(M.port, path)
end

-- A tiny JSON encoder: the payloads are flat and always string-valued.
local function encode(value)
  local parts = {}
  for key, item in pairs(value) do
    local encoded
    if type(item) == "number" then
      encoded = tostring(item)
    elseif type(item) == "boolean" then
      encoded = item and "true" or "false"
    else
      local escaped = tostring(item)
        :gsub("\\", "\\\\")
        :gsub('"', '\\"')
        :gsub("\n", "\\n")
        :gsub("\r", "\\r")
        :gsub("\t", "\\t")
      encoded = '"' .. escaped .. '"'
    end
    table.insert(parts, ('"%s":%s'):format(key, encoded))
  end
  return "{" .. table.concat(parts, ",") .. "}"
end

--- Run a request without blocking the UI. `done` receives (ok, body).
local function request(method, path, body, done)
  local args = { "curl", "-sS", "--max-time", "10", "-X", method, endpoint(path) }
  if body then
    table.insert(args, "-H")
    table.insert(args, "Content-Type: application/json")
    table.insert(args, "-d")
    table.insert(args, body)
  end

  vim.system(args, { text = true }, function(result)
    vim.schedule(function()
      if result.code ~= 0 then
        if done then
          done(false, result.stderr or "request failed")
        else
          notify(result.stderr or "request failed", vim.log.levels.ERROR)
        end
        return
      end
      if done then
        done(true, result.stdout or "")
      end
    end)
  end)
end

local function decode(body)
  local ok, parsed = pcall(vim.json.decode, body)
  if ok then
    return parsed
  end
  return nil
end

--- The app needs to know which problem/language a push belongs to.
local function current_payload(include_code)
  local buffer = vim.api.nvim_get_current_buf()
  local name = vim.api.nvim_buf_get_name(buffer)
  local payload = {}

  -- The workspace layout is <root>/<id>-<slug>/solution.<ext>.
  local slug = name:match("/([^/]+)/solution%.[^/]+$")
  if slug then
    payload.slug = slug:gsub("^%d+%-", "")
  end
  payload.language = vim.bo[buffer].filetype

  if include_code then
    payload.code = table.concat(vim.api.nvim_buf_get_lines(buffer, 0, -1, false), "\n")
  end

  return payload, name
end

function M.status()
  request("GET", "/state", nil, function(ok, body)
    if not ok then
      notify("could not reach Leetr — is it running?", vim.log.levels.WARN)
      return
    end
    local state = decode(body)
    if not state or not state.slug then
      notify("nothing open yet")
      return
    end
    local message = ("%s (%s) · %s · %s"):format(
      state.title or state.slug,
      state.slug,
      state.language or "?",
      state.solved and "solved" or "unsolved"
    )
    local verdict = verdict_text(state)
    if verdict then
      message = message .. "\n" .. verdict
    end
    notify(message)
  end)
end

function M.open(slug)
  if not slug or slug == "" then
    notify("usage: :LeetrOpen <slug>", vim.log.levels.WARN)
    return
  end
  request("POST", "/open", encode({ slug = slug }), function(ok)
    if ok then
      notify("opening " .. slug)
    end
  end)
end

function M.pull()
  request("GET", "/state", nil, function(ok, body)
    if not ok then
      notify("could not reach Leetr", vim.log.levels.WARN)
      return
    end
    local state = decode(body)
    if not state or not state.file then
      notify("Leetr has no file open yet", vim.log.levels.WARN)
      return
    end

    local current = vim.api.nvim_buf_get_name(0)
    if current == state.file then
      -- Same file: take the app's version verbatim.
      local lines = vim.split(state.code, "\n", { plain = true })
      vim.api.nvim_buf_set_lines(0, 0, -1, false, lines)
      notify("buffer updated from Leetr")
    else
      vim.cmd.edit(vim.fn.fnameescape(state.file))
      notify("opened " .. state.file)
    end
  end)
end

--- Render a judge verdict from the app's state, if there is one.
local function verdict_text(state)
  local judge = state and state.judge
  if not judge then
    return nil
  end
  if judge.error then
    return judge.error
  end
  local text = judge.verdict or "?"
  if judge.summary and judge.summary ~= "" and judge.summary ~= judge.verdict then
    text = text .. " — " .. judge.summary
  end
  return text
end

function M.run()
  request("POST", "/run", nil, function(ok)
    if ok then
      notify("running sample tests…")
      M.wait_for_verdict(0)
    end
  end)
end

function M.submit()
  request("POST", "/submit", nil, function(ok)
    if ok then
      notify("submitting…")
      M.wait_for_verdict(0)
    end
  end)
end

function M.result()
  request("GET", "/state", nil, function(ok, body)
    if not ok then
      notify("could not reach Leetr", vim.log.levels.WARN)
      return
    end
    local text = verdict_text(decode(body))
    if not text then
      notify("nothing has been run or submitted yet")
      return
    end
    notify(text)
  end)
end

--- Poll the app until the judge finishes, then report the verdict.
function M.wait_for_verdict(attempts)
  attempts = attempts or 0
  if attempts > 60 then
    notify("still judging — check :LeetrStatus", vim.log.levels.WARN)
    return
  end
  vim.defer_fn(function()
    request("GET", "/state", nil, function(ok, body)
      if not ok then
        return
      end
      local state = decode(body)
      local judge = state and state.judge
      if judge and not judge.running and judge.verdict then
        local accepted = judge.verdict == "Accepted"
        notify(
          verdict_text(state),
          accepted and vim.log.levels.INFO or vim.log.levels.WARN
        )
      else
        M.wait_for_verdict(attempts + 1)
      end
    end)
  end, 1000)
end

function M.push()
  local payload = current_payload(true)
  request("POST", "/code", encode(payload), function(ok)
    if ok then
      notify("pushed to Leetr")
    end
  end)
end

function M.setup(options)
  options = options or {}
  if options.port then
    M.port = options.port
  end

  local auto_push = options.auto_push ~= false

  vim.api.nvim_create_user_command("LeetrStatus", function()
    M.status()
  end, { desc = "Show what Leetr has open" })

  vim.api.nvim_create_user_command("LeetrOpen", function(args)
    M.open(args.args)
  end, { nargs = "?", desc = "Ask Leetr to open a problem" })

  vim.api.nvim_create_user_command("LeetrPull", function()
    M.pull()
  end, { desc = "Load Leetr's buffer into this window" })

  vim.api.nvim_create_user_command("LeetrPush", function()
    M.push()
  end, { desc = "Send this buffer to Leetr" })

  vim.api.nvim_create_user_command("LeetrRun", function()
    M.run()
  end, { desc = "Run the sample tests in Leetr" })

  vim.api.nvim_create_user_command("LeetrSubmit", function()
    M.submit()
  end, { desc = "Submit the current buffer from Leetr" })

  vim.api.nvim_create_user_command("LeetrResult", function()
    M.result()
  end, { desc = "Show the last judge verdict" })

  vim.api.nvim_create_user_command("LeetrPort", function(args)
    M.port = tonumber(args.args) or M.port
    notify("port set to " .. M.port)
  end, { nargs = 1, desc = "Point leetr.nvim at another port" })

  if auto_push then
    vim.api.nvim_create_augroup("Leetr", { clear = true })
    vim.api.nvim_create_autocmd("BufWritePost", {
      group = "Leetr",
      pattern = { "*/solution.*" },
      callback = function()
        local _, name = current_payload(false)
        if name:match("/solution%.[^/]+$") then
          M.push()
        end
      end,
      desc = "Push Leetr workspace files to the app on save",
    })
  end

  return M
end

return M
