local ok, err = require('dojo').apply()
if not ok then
  vim.notify('Dojo theme: ' .. tostring(err), vim.log.levels.WARN)
end
