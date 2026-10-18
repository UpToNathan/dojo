-- Layout for the snacks dashboard: a logo, the key menu, the startup line and a bonsai.
--
-- snacks folds panes that do not fit back onto the first ones, which stacks everything into
-- one tall column in a narrow window and leaves a short logo beside a tall tree in a wide one.
-- This chooses the arrangement for the window instead, on every resize: columns when they fit,
-- a stack when they do not, and the shorter columns centred against the tallest one. The
-- bonsai is the first thing to go when the window is short, then the logo.
local M = {}

M.pane_width = 44
M.pane_gap = 8

local LOGO_COLUMNS = 34
local TREE_ROWS = 25
local MIN_TREE_ROWS = 10
local MARGIN_ROWS = 2 -- the statusline and the cmdline

local function add(list, items)
  for _, item in ipairs(items) do list[#list + 1] = item end
end

local function spacer(rows, pane)
  local items = {}
  for _ = 1, math.max(0, rows) do items[#items + 1] = { text = ' ', pane = pane } end
  return items
end

-- A plain item rather than the `header` section, which adds two rows of padding of its own.
local function logo(pane, header)
  return { { header = header, pane = pane, align = 'center' } }
end

local function rows_of(text)
  return select(2, text:gsub('\n', '\n')) + 1
end

--- The key rows as one block: icon, label and key in fixed columns, so the menu is as wide as its
--- longest label and sits in the middle of its pane, not spread to the pane's edges.
--- @param keys { icon: string?, desc: string, key: string, action: any, hidden: boolean? }[]
--- @param startup table the startup-time line, an item with a `text`
local function menu(pane, keys, startup)
  local label = 0
  for _, key in ipairs(keys) do
    if not key.hidden then label = math.max(label, vim.api.nvim_strwidth(key.desc)) end
  end
  local items = {}
  for _, key in ipairs(keys) do
    local item = { key = key.key, action = key.action, pane = pane, hidden = key.hidden }
    if not key.hidden then
      item.align = 'center'
      item.text = {
        { key.icon or '', width = 3, hl = 'icon' },
        { key.desc, width = label + 3, hl = 'desc' },
        { key.key, hl = 'key' },
      }
    end
    items[#items + 1] = item
  end
  add(items, spacer(1, pane))
  items[#items + 1] = vim.tbl_extend('force', startup, { pane = pane, align = 'center' })
  return items
end

local function visible(keys)
  local count = 0
  for _, key in ipairs(keys) do
    if not key.hidden then count = count + 1 end
  end
  return count
end

local function tree(pane, rows, command, width)
  return { {
    section = 'terminal', cmd = command, pane = pane, height = rows, width = width,
    indent = 0, gap = 0, padding = 0, align = 'center',
  } }
end

local function centred(rows, tallest)
  return math.floor((tallest - rows) / 2)
end

--- @param o { width: integer, height: integer, pane_width: integer, pane_gap: integer, bonsai: string?, keys: table[], header: string, startup: table }
--- @return table[] the dashboard sections, each assigned to a pane
function M.layout(o)
  local panes = math.max(1, math.floor((o.width + o.pane_gap) / (o.pane_width + o.pane_gap)))
  local tree_rows = o.bonsai and math.min(TREE_ROWS, o.height - MARGIN_ROWS) or 0
  local has_tree = tree_rows >= MIN_TREE_ROWS
  local has_logo = o.width >= LOGO_COLUMNS + 2
  local logo_rows = rows_of(o.header)
  local menu_rows = visible(o.keys) + 1 + 1 -- the keys, a blank row, the startup line
  local sections = {}

  if panes >= 3 and has_tree then
    -- logo | bonsai | menu, the outer two centred on the tree
    add(sections, spacer(centred(logo_rows, tree_rows), 1))
    add(sections, logo(1, o.header))
    add(sections, tree(2, tree_rows, o.bonsai, o.pane_width))
    add(sections, spacer(centred(menu_rows, tree_rows), 3))
    add(sections, menu(3, o.keys, o.startup))
  elseif panes >= 2 and has_tree and o.height >= logo_rows + 1 + menu_rows + MARGIN_ROWS then
    -- logo over menu | bonsai, the left column centred on the tree
    local left = logo_rows + 1 + menu_rows
    add(sections, spacer(centred(left, tree_rows), 1))
    add(sections, logo(1, o.header))
    add(sections, spacer(1, 1))
    add(sections, menu(1, o.keys, o.startup))
    add(sections, tree(2, tree_rows, o.bonsai, o.pane_width))
  elseif panes >= 2 and has_logo and o.height >= logo_rows then
    -- no room for the tree: logo | menu
    add(sections, logo(1, o.header))
    add(sections, menu(2, o.keys, o.startup))
  else
    -- one column: menu, then the logo and the bonsai as far as the height allows
    local used = menu_rows
    local show_logo = has_logo and o.height >= used + logo_rows + 1 + MARGIN_ROWS
    if show_logo then used = used + logo_rows + 1 end
    local room = o.height - used - 1 - MARGIN_ROWS
    local rows = o.bonsai and math.min(TREE_ROWS, room) or 0
    if show_logo then
      add(sections, logo(1, o.header))
      add(sections, spacer(1, 1))
    end
    add(sections, menu(1, o.keys, o.startup))
    if rows >= MIN_TREE_ROWS then
      add(sections, spacer(1, 1))
      add(sections, tree(1, rows, o.bonsai, o.pane_width))
    end
  end
  return sections
end

--- The `sections` function for snacks: sizes the layout to the dashboard's window.
--- @param bonsai string? the command that draws the bonsai, or nil to leave it out
function M.sections(bonsai)
  return function(dashboard)
    local size = dashboard._size or { width = vim.o.columns, height = vim.o.lines }
    -- A window narrower than a pane shrinks the pane rather than overflowing it.
    dashboard.opts.width = math.max(24, math.min(M.pane_width, size.width - 2))
    -- `session` keys are resolved by snacks into whatever the installed session plugin loads.
    local keys = {}
    for _, key in ipairs(dashboard.opts.preset.keys) do
      local action = key.action
      if key.section then
        local section = Snacks.dashboard.sections[key.section]
        local resolved = section and section(key)
        action = resolved and resolved.action
      end
      if action then keys[#keys + 1] = vim.tbl_extend('force', key, { action = action }) end
    end
    -- The startup line is the widest thing in the menu; a narrow pane gets the short form.
    local startup = Snacks.dashboard.sections.startup()
    if dashboard.opts.width < M.pane_width - 2 then startup.text[1][1] = '⚡ ' end
    return M.layout({
      startup = startup,
      keys = keys,
      header = dashboard.opts.preset.header,
      width = size.width,
      height = size.height,
      pane_width = dashboard.opts.width,
      pane_gap = dashboard.opts.pane_gap,
      bonsai = bonsai,
    })
  end
end

return M
