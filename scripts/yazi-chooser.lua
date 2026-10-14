-- Private chooser plugin; ordinary Yazi sessions never load these keys.
local cwd = ya.sync(function() return tostring(cx.active.current.cwd) end)

return {
  entry = function()
    local path = cwd()
    if os.getenv("DOJO_CHOOSER_SAVE") == "1" then
      local value, event = ya.input {
        title = "Save as:", value = os.getenv("DOJO_CHOOSER_NAME") or "",
        pos = { "top-center", y = 3, w = 60 },
      }
      -- Cancel the request, not another picker iteration.
      if event ~= 1 or not value or value == "" or value:find("[\r\n]") then
        ya.emit("app:quit", { ["no-cwd-file"] = true })
        return
      end
      path = value:sub(1, 1) == "/" and value or (path .. "/" .. value)
    end
    if not path:find("[\r\n]") then
      fs.write(Url(os.getenv("DOJO_CHOOSER_OUTPUT")), path .. "\n")
    end
    -- Bypass mgr:quit's unfinished-task confirmation: this plugin itself
    -- is an active task. app:quit is the final exit action used by Yazi.
    ya.emit("app:quit", { ["no-cwd-file"] = true })
  end,
}
