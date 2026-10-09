local opt = vim.opt

vim.g.loaded_netrw = 1
vim.g.loaded_netrwPlugin = 1

opt.mouse = 'a'
opt.mousemodel = 'popup_setpos'
opt.clipboard = 'unnamedplus'
opt.swapfile = false
opt.completeopt = 'menuone,noselect'

opt.number = true
opt.termguicolors = true
opt.signcolumn = 'yes'
opt.showmode = false
opt.laststatus = 3
opt.cursorline = true

opt.expandtab = true
opt.tabstop = 2
opt.shiftwidth = 2
opt.softtabstop = 2
opt.autoindent = true
opt.smartindent = true
opt.linebreak = true
opt.smoothscroll = true
opt.splitkeep = 'screen'

opt.ignorecase = true
opt.smartcase = true
opt.hlsearch = false

opt.splitbelow = true
opt.splitright = true
opt.mousemoveevent = true

opt.foldcolumn = '1'
opt.foldlevel = 99
opt.foldlevelstart = 99
opt.foldenable = true
opt.foldmethod = 'expr'
opt.foldexpr = 'v:lua.vim.lsp.foldexpr()'
opt.foldtext = 'v:lua.vim.lsp.foldtext()'

vim.filetype.add({
  extension = { mdx = 'markdown' },
})
vim.g.markdown_fenced_languages = { 'ts=typescript' }

local function refresh_popup_menu()
  vim.cmd('silent! aunmenu PopUp.LSP')
  vim.cmd('silent! aunmenu PopUp.Rename')
  vim.cmd('silent! aunmenu PopUp.Code\\ Actions')
  vim.cmd('silent! aunmenu PopUp.Find\\ References')
  vim.cmd('silent! aunmenu PopUp.Format')
  vim.cmd('silent! aunmenu PopUp.Git')
  vim.cmd('silent! aunmenu PopUp.Review')
  vim.cmd('silent! aunmenu PopUp.Debug')

  local bufnr = vim.api.nvim_get_current_buf()
  local ft = vim.bo[bufnr].filetype
  local clients = vim.lsp.get_clients({ bufnr = bufnr })
  local function supports(method)
    for _, client in ipairs(clients) do
      if client:supports_method(method, bufnr) then
        return true
      end
    end
    return false
  end

  if supports('textDocument/rename') then
    vim.cmd([[amenu PopUp.Rename <Cmd>lua vim.lsp.buf.rename()<CR>]])
  end
  if supports('textDocument/codeAction') then
    vim.cmd([[amenu PopUp.Code\ Actions <Cmd>lua vim.lsp.buf.code_action()<CR>]])
  end
  if supports('textDocument/references') then
    vim.cmd([[amenu PopUp.Find\ References <Cmd>lua vim.lsp.buf.references()<CR>]])
  end
  if supports('textDocument/formatting') then
    vim.cmd([[amenu PopUp.Format <Cmd>lua vim.lsp.buf.format({ async = true })<CR>]])
  end

  local ok, gitsigns = pcall(require, 'gitsigns')
  local hunks = ok and gitsigns.get_hunks(bufnr) or nil
  if hunks and #hunks > 0 then
    vim.cmd([[amenu PopUp.Git.Preview\ Hunk <Cmd>lua require('gitsigns').preview_hunk()<CR>]])
    vim.cmd([[amenu PopUp.Git.Stage\ Hunk <Cmd>lua require('gitsigns').stage_hunk()<CR>]])
    vim.cmd([[amenu PopUp.Git.Reset\ Hunk <Cmd>lua require('gitsigns').reset_hunk()<CR>]])
    vim.cmd([[amenu PopUp.Git.Blame\ Line <Cmd>lua require('gitsigns').blame_line()<CR>]])
  end

  if vim.fn.executable('hunk') == 1 then
    vim.cmd([[amenu PopUp.Review.Working\ Tree <Cmd>lua Snacks.terminal({ 'hunk', 'diff' })<CR>]])
    vim.cmd([[amenu PopUp.Review.Staged\ Changes <Cmd>lua Snacks.terminal({ 'hunk', 'diff', '--staged' })<CR>]])
  end

  local dap_ok, dap = pcall(require, 'dap')
  if dap_ok and dap.configurations[ft] then
    vim.cmd([[amenu PopUp.Debug.Toggle\ Breakpoint <Cmd>lua require('dap').toggle_breakpoint()<CR>]])
    vim.cmd([[amenu PopUp.Debug.Continue <Cmd>lua require('dap').continue()<CR>]])
  end
end

vim.api.nvim_create_autocmd({ 'BufEnter', 'BufWritePost', 'DiagnosticChanged', 'LspAttach', 'LspDetach' }, {
  callback = refresh_popup_menu,
})

vim.env.PATH = vim.env.HOME .. '/.local/bin:' .. vim.env.HOME .. '/.local/npm/bin:' .. vim.env.PATH
refresh_popup_menu()
