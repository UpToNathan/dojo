local M = {}
local timer
local last_contents

local function palette_path()
  local state = vim.env.XDG_STATE_HOME or (vim.env.HOME .. '/.local/state')
  return state .. '/dojo/themes/current/nvim.json'
end

local function read_palette()
  local file, err = io.open(palette_path(), 'rb')
  if not file then return nil, err end
  local contents = file:read('*a')
  file:close()
  local ok, palette = pcall(vim.json.decode, contents)
  if not ok or type(palette) ~= 'table' or type(palette.colors) ~= 'table' then
    return nil, 'invalid palette JSON'
  end
  for _, name in ipairs({ 'background', 'foreground', 'surface', 'surface_raised',
    'background_highlight', 'border', 'foreground_muted', 'primary', 'secondary',
    'primary_foreground', 'selection_foreground', 'error', 'warning', 'success', 'info',
    'diff_added_background', 'diff_removed_background' }) do
    if type(palette.colors[name]) ~= 'string' or not palette.colors[name]:match('^#%x%x%x%x%x%x$') then
      return nil, 'invalid palette role ' .. name
    end
  end
  if palette.appearance ~= 'light' and palette.appearance ~= 'dark' then
    return nil, 'invalid appearance'
  end
  if palette.preset ~= nil and palette.preset ~= vim.NIL and type(palette.preset) ~= 'string' then
    return nil, 'invalid preset metadata'
  end
  for name, color in pairs(palette.colors) do
    if type(color) ~= 'string' or not color:match('^#%x%x%x%x%x%x$') then
      return nil, 'invalid palette role ' .. name
    end
  end
  if type(palette.terminal) ~= 'table' or #palette.terminal ~= 16 then
    return nil, 'invalid terminal palette'
  end
  for _, color in ipairs(palette.terminal) do
    if type(color) ~= 'string' or not color:match('^#%x%x%x%x%x%x$') then
      return nil, 'invalid terminal color'
    end
  end
  return palette, contents
end

local function apply_native_preset(preset)
  if type(preset) ~= 'string' then return false end
  local family, variant
  if preset:match('^tokyo%-night%-') then
    family, variant = 'tokyonight', preset:match('^tokyo%-night%-(%a+)$')
    if not ({ night = true, storm = true, moon = true, day = true })[variant] then
      return nil, 'unknown Tokyo Night variant'
    end
  elseif preset:match('^catppuccin%-') then
    family, variant = 'catppuccin', preset:match('^catppuccin%-(%a+)$')
    if not ({ latte = true, frappe = true, macchiato = true, mocha = true })[variant] then
      return nil, 'unknown Catppuccin variant'
    end
  else
    return false
  end
  local config = vim.env.XDG_CONFIG_HOME or (vim.env.HOME .. '/.config')
  local file, err = io.open(config .. '/dojo/nvim-presets.json', 'rb')
  if not file then return nil, 'native preset plugins need Home Manager provisioning: ' .. tostring(err) end
  local contents = file:read('*a')
  file:close()
  local ok, paths = pcall(vim.json.decode, contents)
  if not ok or type(paths) ~= 'table' or type(paths[family]) ~= 'string' or paths[family]:sub(1, 1) ~= '/' then
    return nil, 'invalid native preset plugin paths'
  end
  vim.opt.runtimepath:prepend(paths[family])
  local applied, failure = pcall(function()
    if family == 'tokyonight' then
      require('tokyonight').setup({ style = variant, light_style = 'night' })
      require('tokyonight').load({ style = variant })
    else
      require('catppuccin').setup({ flavour = variant })
      require('catppuccin').load()
    end
  end)
  if not applied then return nil, tostring(failure) end
  return true
end

function M.apply()
  local palette, contents = read_palette()
  if not palette then return false, contents end
  local native, err = apply_native_preset(palette.preset)
  if native == nil then return false, err end
  if native then
    vim.g.colors_name = 'dojo'
    vim.g.dojo_native_preset = palette.preset
    last_contents = contents
    return true
  end
  local c = palette.colors
  vim.cmd('highlight clear')
  vim.g.dojo_native_preset = nil
  vim.o.termguicolors = true
  vim.o.background = palette.appearance
  vim.g.colors_name = 'dojo'
  for index, color in ipairs(palette.terminal) do vim.g['terminal_color_' .. (index - 1)] = color end
  local groups = {
    Normal = { fg = c.foreground, bg = c.background },
    NormalNC = { fg = c.foreground, bg = c.background },
    NormalFloat = { fg = c.foreground, bg = c.surface },
    FloatBorder = { fg = c.float_border or c.border, bg = c.surface },
    FloatTitle = { fg = c.primary, bg = c.surface, bold = true },
    WinSeparator = { fg = c.border },
    SignColumn = { fg = c.line_number or c.foreground_muted, bg = c.background },
    LineNr = { fg = c.line_number or c.foreground_muted },
    CursorLineNr = { fg = c.syntax_number or c.primary, bold = true },
    CursorLine = { bg = c.surface_raised },
    CursorColumn = { bg = c.surface_raised },
    ColorColumn = { bg = c.surface },
    Visual = { fg = c.selection_foreground, bg = c.background_highlight },
    Search = { fg = c.primary_foreground, bg = c.primary },
    IncSearch = { fg = c.primary_foreground, bg = c.primary, bold = true },
    MatchParen = { fg = c.info, bold = true, underline = true },
    Pmenu = { fg = c.foreground, bg = c.surface },
    PmenuSel = { fg = c.selection_foreground, bg = c.background_highlight },
    PmenuSbar = { bg = c.surface_raised }, PmenuThumb = { bg = c.border },
    StatusLine = { fg = c.foreground, bg = c.surface },
    StatusLineNC = { fg = c.foreground_muted, bg = c.surface },
    TabLine = { fg = c.foreground_muted, bg = c.surface },
    TabLineSel = { fg = c.primary, bg = c.surface_raised, bold = true },
    TabLineFill = { bg = c.background },
    Folded = { fg = c.foreground_muted, bg = c.surface },
    EndOfBuffer = { fg = c.border }, NonText = { fg = c.border },
    Whitespace = { fg = c.border }, Conceal = { fg = c.foreground_muted },
    Comment = { fg = c.syntax_comment or c.foreground_muted, italic = true },
    Constant = { fg = c.syntax_number or c.warning }, String = { fg = c.syntax_string or c.success },
    Number = { fg = c.syntax_number or c.warning }, Boolean = { fg = c.syntax_number or c.warning },
    Identifier = { fg = c.syntax_identifier or c.syntax_variable or c.foreground }, Function = { fg = c.syntax_function or c.primary },
    Statement = { fg = c.syntax_statement or c.syntax_keyword or c.secondary }, Keyword = { fg = c.syntax_keyword or c.secondary },
    Operator = { fg = c.syntax_operator or c.info }, Type = { fg = c.syntax_type or c.info },
    PreProc = { fg = c.syntax_keyword or c.secondary }, Special = { fg = c.syntax_type or c.info },
    Delimiter = { fg = c.syntax_punctuation or c.foreground_muted }, Underlined = { fg = c.info, underline = true },
    Title = { fg = c.primary, bold = true }, Directory = { fg = c.primary },
    ErrorMsg = { fg = c.error }, WarningMsg = { fg = c.warning },
    DiffAdd = { bg = c.diff_added_background }, DiffDelete = { bg = c.diff_removed_background },
    DiffChange = { bg = c.surface_raised }, DiffText = { bg = c.background_highlight },
  }
  for group, highlight in pairs(groups) do vim.api.nvim_set_hl(0, group, highlight) end
  local links = {
    ['@comment'] = 'Comment', ['@string'] = 'String', ['@number'] = 'Number',
    ['@boolean'] = 'Boolean', ['@constant'] = 'Constant', ['@variable'] = 'Identifier',
    ['@variable.parameter'] = 'Identifier', ['@variable.member'] = 'Identifier',
    ['@function'] = 'Function', ['@function.call'] = 'Function', ['@method'] = 'Function',
    ['@keyword'] = 'Keyword', ['@operator'] = 'Operator', ['@type'] = 'Type',
    ['@type.builtin'] = 'Type', ['@punctuation.delimiter'] = 'Delimiter',
    ['@punctuation.bracket'] = 'Delimiter', ['@tag'] = 'Type', ['@tag.attribute'] = 'Identifier',
    ['@markup.heading'] = 'Title', ['@markup.raw'] = 'String', ['@markup.link'] = 'Underlined',
    ['@lsp.type.class'] = 'Type', ['@lsp.type.interface'] = 'Type', ['@lsp.type.type'] = 'Type',
    ['@lsp.type.function'] = 'Function', ['@lsp.type.method'] = 'Function',
    ['@lsp.type.variable'] = 'Identifier', ['@lsp.type.parameter'] = 'Identifier',
    ['@lsp.type.property'] = 'Identifier', ['@lsp.type.enumMember'] = 'Constant',
    SnacksNormal = 'NormalFloat', SnacksBorder = 'FloatBorder', SnacksTitle = 'FloatTitle',
    SnacksPickerNormal = 'NormalFloat', SnacksPickerBorder = 'FloatBorder',
    SnacksPickerTitle = 'FloatTitle', SnacksPickerMatch = 'Search',
    SnacksPickerListCursorLine = 'PmenuSel', SnacksPickerInput = 'NormalFloat',
    SnacksDashboardNormal = 'Normal', SnacksDashboardHeader = 'Title',
    SnacksDashboardFooter = 'Comment', SnacksDashboardKey = 'Function',
    WhichKey = 'Function', WhichKeyGroup = 'Type', WhichKeyDesc = 'NormalFloat',
    WhichKeyBorder = 'FloatBorder', WhichKeyNormal = 'NormalFloat',
    FlashLabel = 'IncSearch', FlashMatch = 'Search', FlashBackdrop = 'Comment',
    TroubleNormal = 'NormalFloat', TroubleText = 'Normal', TroubleCount = 'Number',
    NoicePopup = 'NormalFloat', NoicePopupBorder = 'FloatBorder', NoiceCmdlinePopup = 'NormalFloat',
    BlinkCmpMenu = 'Pmenu', BlinkCmpMenuBorder = 'FloatBorder', BlinkCmpMenuSelection = 'PmenuSel',
    BlinkCmpDoc = 'NormalFloat', BlinkCmpDocBorder = 'FloatBorder',
    FffNormal = 'NormalFloat', FffBorder = 'FloatBorder', FffTitle = 'FloatTitle',
    FffSelected = 'PmenuSel', FffMatch = 'Search',
    DapUIPlayPause = 'Function', DapUIScope = 'Type', DapUIType = 'Type',
    DapUIVariable = 'Identifier', DapUIValue = 'String', OutlineNormal = 'NormalFloat',
    OutlineGuides = 'Comment', GrappleNormal = 'NormalFloat', GrappleBorder = 'FloatBorder',
  }
  for group, target in pairs(links) do vim.api.nvim_set_hl(0, group, { link = target }) end
  if c.syntax_variable then
    vim.api.nvim_set_hl(0, '@variable', { fg = c.syntax_variable })
    for _, group in ipairs({ '@variable.parameter', '@variable.member', '@lsp.type.variable', '@lsp.type.parameter', '@lsp.type.property' }) do
      vim.api.nvim_set_hl(0, group, { link = '@variable' })
    end
  end
  for name, role in pairs({ Error = 'error', Warn = 'warning', Info = 'info', Hint = 'info', Ok = 'success' }) do
    vim.api.nvim_set_hl(0, 'Diagnostic' .. name, { fg = c[role] })
    vim.api.nvim_set_hl(0, 'DiagnosticVirtualText' .. name, { fg = c[role], bg = c.surface })
    vim.api.nvim_set_hl(0, 'DiagnosticUnderline' .. name, { sp = c[role], undercurl = true })
    vim.api.nvim_set_hl(0, 'DiagnosticSign' .. name, { fg = c[role] })
  end
  for name, role in pairs({ Add = 'success', Change = 'warning', Delete = 'error' }) do
    vim.api.nvim_set_hl(0, 'GitSigns' .. name, { fg = c[role] })
  end
  last_contents = contents
  return true
end

function M.reload()
  if vim.g.colors_name == 'dojo' then
    vim.cmd.colorscheme('dojo')
    vim.cmd.redraw()
  end
end

function M.setup()
  if timer then return end
  if read_palette() then vim.cmd.colorscheme('dojo') end
  vim.api.nvim_create_user_command('DojoThemeReload', M.reload, {})
  timer = vim.uv.new_timer()
  timer:start(1000, 1000, vim.schedule_wrap(function()
    if vim.g.colors_name ~= 'dojo' then return end
    local palette, contents = read_palette()
    if palette and contents ~= last_contents then M.reload() end
  end))
  vim.api.nvim_create_autocmd('VimLeavePre', { once = true, callback = function()
    timer:stop()
    timer:close()
    timer = nil
  end })
end

return M
