-- Run: nvim --headless -l tests/nvim_dashboard.lua
-- Checks the snacks dashboard layout (nvim-config/lua/dojo/dashboard.lua) over a grid of window
-- sizes. snacks appends every pane after the first to rows the earlier panes already made, so
-- the middle pane must be the tallest; the rest is what keeps the layout balanced.
local root = vim.fn.fnamemodify(debug.getinfo(1, 'S').source:sub(2), ':p:h:h')
package.path = root .. '/nvim-config/lua/?.lua;' .. package.path
local dashboard = require('dojo.dashboard')

local header = table.concat({ 'AAAA', 'BBBB', 'CCCC', 'DDDD', 'EEEE', 'FFFF', ' ' }, '\n')
local keys = {}
for _, name in ipairs({ 'Recent files', 'Config', 'Restore session', 'Lazy', 'Update' }) do
  keys[#keys + 1] = { icon = 'x', desc = name, key = name:sub(1, 1):lower(), action = ':noop' }
end
keys[#keys + 1] = { key = 'q', desc = 'Quit', action = ':qa', hidden = true }
local startup = { text = { { 'startup' } } }

local function rows(item)
  if item.hidden then return 0 end
  if item.section == 'terminal' then return item.height end
  if item.header then return select(2, item.header:gsub('\n', '\n')) + 1 end
  return 1
end

local function measure(sections)
  local panes = {}
  for _, item in ipairs(sections) do
    local pane = item.pane or 1
    panes[pane] = panes[pane] or { rows = 0, top = 0, has = {} }
    local entry = panes[pane]
    if item.text == ' ' then
      if not entry.seen then entry.top = entry.top + 1 end -- padding above the first real item
    else
      entry.seen = true
    end
    entry.rows = entry.rows + rows(item)
    entry.has[#entry.has + 1] = item
  end
  return panes
end

local failures = 0
local function check(ok, message)
  if not ok then
    failures = failures + 1
    print('FAIL ' .. message)
  end
end

for _, width in ipairs({ 20, 30, 36, 40, 60, 95, 96, 100, 147, 148, 200, 300 }) do
  for _, height in ipairs({ 6, 8, 12, 14, 20, 27, 30, 45, 70 }) do
    local pane_width = math.max(24, math.min(dashboard.pane_width, width - 2))
    local sections = dashboard.layout({
      width = width, height = height, pane_width = pane_width, pane_gap = dashboard.pane_gap,
      bonsai = 'bonsai', keys = keys, header = header, startup = startup,
    })
    local where = ('%dx%d'):format(width, height)
    local panes = measure(sections)
    local count = 0
    for index in pairs(panes) do count = math.max(count, index) end
    local fits = math.max(1, math.floor((width + dashboard.pane_gap) / (pane_width + dashboard.pane_gap)))

    check(count <= fits, where .. ': uses more panes than fit')
    for index = 1, count do
      check(panes[index] ~= nil, where .. ': pane ' .. index .. ' is empty but a later pane is used')
    end
    if height >= 9 then
      for index = 1, count do
        check(panes[index].rows <= height, where .. (': pane %d is %d rows'):format(index, panes[index].rows))
      end
    end
    local has_tree = false
    for _, item in ipairs(sections) do
      if item.section == 'terminal' then has_tree = true end
    end
    local has_logo = false
    for _, item in ipairs(sections) do
      if item.header then has_logo = true end
    end
    check(not has_tree or has_logo or width < 36 or height < 20, where .. ': the tree is kept over the logo')
    check(not (has_tree and height < 12), where .. ': a tree in a window too short for it')

    if count >= 2 and has_tree then
      -- The tree's pane is the tallest, and the other columns are centred against it.
      local tree_pane
      for index = 1, count do
        for _, item in ipairs(panes[index].has) do
          if item.section == 'terminal' then tree_pane = index end
        end
      end
      for index = 1, count do
        if index ~= tree_pane then
          check(panes[tree_pane].rows >= panes[index].rows, where .. ': a column taller than the tree')
        end
      end
      if count == 3 then check(tree_pane == 2, where .. ': the tree is not the middle pane') end
      for index = 1, count do
        if index ~= tree_pane then
          local below = panes[tree_pane].rows - panes[index].rows
          check(math.abs(below - panes[index].top) <= 1, where .. (': pane %d is off centre (%d above, %d below)'):format(index, panes[index].top, below))
        end
      end
    end
  end
end

-- Whether a tier is chosen as designed at a few representative sizes.
local function shape(width, height)
  local sections = dashboard.layout({
    width = width, height = height, pane_width = dashboard.pane_width, pane_gap = dashboard.pane_gap,
    bonsai = 'bonsai', keys = keys, header = header, startup = startup,
  })
  local panes = measure(sections)
  local count = 0
  for index in pairs(panes) do count = math.max(count, index) end
  return count
end
check(shape(200, 45) == 3, 'a wide window gets three columns')
check(shape(110, 45) == 2, 'a medium window gets two columns')
check(shape(80, 45) == 1, 'a narrow window gets one column')
check(shape(200, 10) == 2, 'a wide, short window drops the tree and keeps logo | menu')
check(shape(300, 70) == 3, 'an expanded window keeps the same three columns, centred')

if failures > 0 then
  print(failures .. ' dashboard layout checks failed')
  os.exit(1)
end
print('dashboard layout: all checks passed')
