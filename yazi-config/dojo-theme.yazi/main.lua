local M = {}

function M.setup()
  ps.sub_remote('dojo-theme', function()
    ya.emit('app:theme', {})
  end)
end

return M
