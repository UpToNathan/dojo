vim.lsp.config('lua_ls', {
  cmd = { 'lua-language-server' },
  filetypes = { 'lua' },
  root_markers = { '.luarc.json', '.luarc.jsonc', '.git' },
  settings = {
    Lua = {
      runtime = { version = 'LuaJIT' },
      workspace = { checkThirdParty = false },
      diagnostics = { globals = { 'vim' } },
      telemetry = { enable = false },
    },
  },
})

local servers = {
  'lua_ls',
  'tsc',
  'html',
  'cssls',
  'cssmodules_ls',
  'css_variables',
  'tailwindcss',
  'bashls',
  'fish_lsp',
  'just',
  'gopls',
  'crystalline',
  'jsonls',
  'yamlls',
  'dockerls',
  'docker_compose_language_service',
  'gh_actions_ls',
  'systemd_lsp',
  'denols',
  'oxlint',
  'sqruff',
  'taplo',
}

for _, server in ipairs(servers) do
  vim.lsp.enable(server)
end

vim.api.nvim_create_autocmd('LspAttach', {
  callback = function(args)
    local client = vim.lsp.get_client_by_id(args.data.client_id)
    local buffer = args.buf

    local function map(key, action, description, method)
      if not method or client:supports_method(method, buffer) then
        vim.keymap.set('n', key, action, { buffer = buffer, desc = description })
      end
    end

    map('<leader>lh', vim.lsp.buf.hover, 'Hover', 'textDocument/hover')
    map('<leader>ld', vim.lsp.buf.definition, 'Definition', 'textDocument/definition')
    map('<leader>lD', vim.lsp.buf.declaration, 'Declaration', 'textDocument/declaration')
    map('<leader>li', vim.lsp.buf.implementation, 'Implementation', 'textDocument/implementation')
    map('<leader>lt', vim.lsp.buf.type_definition, 'Type definition', 'textDocument/typeDefinition')
    map('<leader>lr', vim.lsp.buf.references, 'References', 'textDocument/references')
    map('<leader>la', vim.lsp.buf.code_action, 'Code action', 'textDocument/codeAction')
    map('<leader>ln', vim.lsp.buf.rename, 'Rename', 'textDocument/rename')
    map('<leader>ls', vim.lsp.buf.signature_help, 'Signature help', 'textDocument/signatureHelp')

    map('<leader>dd', vim.diagnostic.open_float, 'Show diagnostic')
    map('<leader>dn', vim.diagnostic.goto_next, 'Next diagnostic')
    map('<leader>dp', vim.diagnostic.goto_prev, 'Previous diagnostic')
    map('<leader>dq', vim.diagnostic.setloclist, 'Diagnostics list')

    map('<leader>wa', vim.lsp.buf.add_workspace_folder, 'Add workspace folder', 'workspace/addWorkspaceFolder')
    map('<leader>wr', vim.lsp.buf.remove_workspace_folder, 'Remove workspace folder', 'workspace/removeWorkspaceFolder')
    map('<leader>wl', function() print(vim.inspect(vim.lsp.buf.list_workspace_folders())) end, 'List workspace folders', 'workspace/workspaceFolders')
  end,
})
