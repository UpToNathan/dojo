-- NINJAVIM
vim = vim
require('core.options')

vim.g.mapleader = ' '
vim.g.maplocalleader = '\\'

local lazypath = vim.fn.stdpath('data') .. '/lazy/lazy.nvim'
if not (vim.uv or vim.loop).fs_stat(lazypath) then
  local output = vim.fn.system({
    'git', 'clone', '--filter=blob:none', '--branch=stable',
    'https://github.com/folke/lazy.nvim.git', lazypath,
  })
  if vim.v.shell_error ~= 0 then
    vim.api.nvim_echo({
      { 'Failed to clone lazy.nvim:\n', 'ErrorMsg' },
      { output, 'WarningMsg' },
    }, true, {})
    return
  end
end
vim.opt.rtp:prepend(lazypath)

-- The dashboard's animation. A terminal started from the desktop session has no ~/.cargo/bin on
-- PATH, where `cargo install` puts it, so look there too and leave the panel out if it is absent.
local function rbonsai_command()
  local found = vim.fn.exepath('rbonsai')
  if found == '' then
    local installed = vim.fn.expand('~/.cargo/bin/rbonsai')
    if vim.fn.executable(installed) == 1 then found = installed end
  end
  if found == '' then return nil end
  return vim.fn.shellescape(found) .. ' -p -i -l -L 25'
end
local rbonsai = rbonsai_command()

require('lazy').setup({
  {
    'folke/which-key.nvim',
    event = 'VeryLazy',
    opts = {
      spec = {
        { '<leader>l', group = 'LSP' },
        { '<leader>d', group = 'Diagnostics' },
        { '<leader>w', group = 'Workspace' },
        { '<leader>a', group = 'AI' },
        { '<leader>f', group = 'Find' },
        { '<leader>n', group = 'Notifications' },
        { '<leader>q', group = 'Sessions' },
        { '<leader>u', group = 'UI toggles' },
        { '<leader>x', group = 'Trouble' },
        { '<leader>m', group = 'Marks' },
        { '<leader>g', group = 'Git' },
        { '<leader>D', group = 'Debug' },
        { '<leader>h', group = 'Hunk' },
      },
    },
    config = function(_, opts)
      require('which-key').setup(opts)
    end,
  },
  {
    'dmtrKovalenko/fff',
    lazy = false,
    build = function()
      require('fff.download').download_or_build_binary()
    end,
    opts = {
      debug = {
        enabled = true,
        show_scores = true,
      },
    },
    keys = {
      { '<leader>ff', function() require('fff').find_files() end, desc = 'Find files' },
      { '<leader>fg', function() require('fff').live_grep() end, desc = 'Live grep' },
      { '<leader>fG', function() require('fff').live_grep({ grep = { modes = { 'fuzzy', 'plain' } } }) end, desc = 'Fuzzy grep' },
      { '<leader>fw', function() require('fff').live_grep_under_cursor() end, mode = { 'n', 'x' }, desc = 'Search word or selection' },
    },
  },
  {
    'folke/snacks.nvim',
    priority = 1000,
    lazy = false,
    opts = {
      animate = { enabled = true },
      bigfile = { enabled = true },
      dashboard = {
        width = require('dojo.dashboard').pane_width,
        pane_gap = require('dojo.dashboard').pane_gap,
        preset = {
          header = [[
██████╗  ██████╗      ██╗ ██████╗ 
██╔══██╗██╔═══██╗     ██║██╔═══██╗
██║  ██║██║   ██║     ██║██║   ██║
██║  ██║██║   ██║██   ██║██║   ██║
██████╔╝╚██████╔╝╚█████╔╝╚██████╔╝
╚═════╝  ╚═════╝  ╚════╝  ╚═════╝ 
                                  ]],
          keys = {
            { icon = ' ', key = 'r', desc = 'Recent files', action = ":lua Snacks.dashboard.pick('oldfiles')" },
            { icon = ' ', key = 'c', desc = 'Config', action = ":lua Snacks.dashboard.pick('files', {cwd = vim.fn.stdpath('config')})" },
            { icon = ' ', key = 'p', desc = 'Restore session', section = 'session' },
            { icon = '󰒲 ', key = 'l', desc = 'Lazy', action = ':Lazy' },
            { icon = '󰊳 ', key = 'u', desc = 'Update', action = ':Lazy update' },
            { key = 'q', desc = 'Quit', action = ':qa', hidden = true },
          },
        },
        sections = require('dojo.dashboard').sections(rbonsai),
      },
      explorer = { enabled = true },
      indent = { enabled = true },
      input = { enabled = true },
      lazygit = { enabled = true },
      notifier = { enabled = true },
      picker = { enabled = true },
      quickfile = { enabled = true },
      scope = { enabled = true },
      scroll = { enabled = true },
      scratch = { enabled = true },
      terminal = { enabled = true },
      toggle = { enabled = true },
      words = { enabled = true },
      zen = { enabled = true },
    },
    keys = {
      { '<leader>bh', function() Snacks.dashboard() end, desc = 'Dashboard' },
      { '<leader>e', function() Snacks.explorer() end, desc = 'Explorer' },
      { '<leader>fb', function() Snacks.picker.buffers() end, desc = 'Buffers' },
      { '<leader>fr', function() Snacks.picker.recent() end, desc = 'Recent files' },
      { '<leader>fh', function() Snacks.picker.help() end, desc = 'Help' },
      { '<leader>gg', function() Snacks.lazygit() end, desc = 'Lazygit' },
      { '<leader>gt', function() Snacks.terminal() end, desc = 'Terminal' },
      { '<leader>.', function() Snacks.scratch() end, desc = 'Scratch buffer' },
      { '<leader>z', function() Snacks.zen() end, desc = 'Zen mode' },
      { '<leader>nh', function() Snacks.notifier.show_history() end, desc = 'Notification history' },
      { '<leader>ud', function() Snacks.toggle.diagnostics():toggle() end, desc = 'Toggle diagnostics' },
      { '<leader>us', function() Snacks.toggle.option('spell'):toggle() end, desc = 'Toggle spelling' },
      { '<leader>uw', function() Snacks.toggle.option('wrap'):toggle() end, desc = 'Toggle wrap' },
    },
  },
  {
    'folke/flash.nvim',
    event = 'VeryLazy',
    opts = {},
    keys = {
      { 's', mode = { 'n', 'x', 'o' }, function() require('flash').jump() end, desc = 'Flash' },
      { 'S', mode = { 'n', 'x', 'o' }, function() require('flash').treesitter() end, desc = 'Flash Treesitter' },
      { 'r', mode = 'o', function() require('flash').remote() end, desc = 'Remote Flash' },
      { 'R', mode = { 'o', 'x' }, function() require('flash').treesitter_search() end, desc = 'Treesitter search' },
      { '<C-s>', mode = 'c', function() require('flash').toggle() end, desc = 'Toggle Flash search' },
    },
  },
  {
    'folke/trouble.nvim',
    cmd = 'Trouble',
    opts = {},
    keys = {
      { '<leader>xx', '<cmd>Trouble diagnostics toggle<cr>', desc = 'Diagnostics' },
      { '<leader>xX', '<cmd>Trouble diagnostics toggle filter.buf=0<cr>', desc = 'Buffer diagnostics' },
      { '<leader>cs', '<cmd>Trouble symbols toggle focus=false<cr>', desc = 'Symbols' },
      { '<leader>cl', '<cmd>Trouble lsp toggle focus=false win.position=right<cr>', desc = 'LSP list' },
      { '<leader>xL', '<cmd>Trouble loclist toggle<cr>', desc = 'Location list' },
      { '<leader>xQ', '<cmd>Trouble qflist toggle<cr>', desc = 'Quickfix list' },
    },
  },
  {
    'folke/persistence.nvim',
    event = { 'BufReadPre', 'BufNewFile' },
    opts = { branch = false },
    keys = {
      { '<leader>qs', function() require('persistence').load() end, desc = 'Load session' },
      { '<leader>qS', function() require('persistence').select() end, desc = 'Select session' },
      { '<leader>ql', function() require('persistence').load({ last = true }) end, desc = 'Load last session' },
      { '<leader>qd', function() require('persistence').stop() end, desc = 'Stop session saving' },
    },
  },
  {
    'folke/noice.nvim',
    event = 'VeryLazy',
    dependencies = { 'MunifTanjim/nui.nvim', 'rcarriga/nvim-notify' },
    opts = {
      presets = {
        command_palette = false,
        long_message_to_split = true,
        inc_rename = true,
        lsp_doc_border = false,
      },
    },
    keys = {
      { '<leader>nl', '<cmd>Noice history<cr>', desc = 'Message history' },
      { '<leader>nd', '<cmd>Noice dismiss<cr>', desc = 'Dismiss messages' },
    },
  },
  {
    'folke/lazydev.nvim',
    ft = 'lua',
    opts = {},
  },

  {
    'stevearc/conform.nvim',
    cmd = 'ConformInfo',
    keys = {
      {
        '<leader>lf',
        function()
          for _, client in ipairs(vim.lsp.get_clients({ bufnr = 0 })) do
            if client:supports_method('textDocument/formatting', 0) then
              return vim.lsp.buf.format({ async = true })
            end
          end
          require('conform').format({ async = true, lsp_format = 'never' })
        end,
        mode = { 'n', 'v' },
        desc = 'Format',
      },
    },
    opts = {
      formatters_by_ft = {
        javascript = { 'deno_fmt' },
        javascriptreact = { 'deno_fmt' },
        typescript = { 'deno_fmt' },
        typescriptreact = { 'deno_fmt' },
        json = { 'deno_fmt' },
        jsonc = { 'deno_fmt' },
        go = { 'gofmt' },
        rust = { 'rustfmt' },
        sql = { 'sqruff' },
        toml = { 'taplo' },
      },
    },
  },
  {
    'tris203/precognition.nvim',
    event = { 'BufReadPre', 'BufNewFile' },
    opts = {},
  },
  {
    'nvzone/showkeys',
    cmd = 'ShowkeysToggle',
    keys = {
      { '<leader>uk', '<cmd>ShowkeysToggle<cr>', desc = 'Toggle key display' },
    },
  },
  {
    'hedyhli/outline.nvim',
    cmd = 'Outline',
    keys = {
      { '<leader>lo', '<cmd>Outline<cr>', desc = 'Toggle outline' },
    },
    opts = {},
  },
  {
    'soulis-1256/eagle.nvim',
    event = 'LspAttach',
    opts = {},
  },
  {
    'cbochs/grapple.nvim',
    event = { 'BufReadPost', 'BufNewFile' },
    cmd = 'Grapple',
    dependencies = { 'nvim-tree/nvim-web-devicons' },
    opts = { scope = 'git_branch' },
    keys = {
      { '<leader>m', '<cmd>Grapple toggle<cr>', desc = 'Toggle file mark' },
      { '<leader>M', '<cmd>Grapple toggle_tags<cr>', desc = 'Open file marks' },
      { '<leader>n', '<cmd>Grapple cycle_tags next<cr>', desc = 'Next file mark' },
      { '<leader>p', '<cmd>Grapple cycle_tags prev<cr>', desc = 'Previous file mark' },
      { '<leader>1', '<cmd>Grapple select index=1<cr>', desc = 'Select first file mark' },
      { '<leader>2', '<cmd>Grapple select index=2<cr>', desc = 'Select second file mark' },
      { '<leader>3', '<cmd>Grapple select index=3<cr>', desc = 'Select third file mark' },
      { '<leader>4', '<cmd>Grapple select index=4<cr>', desc = 'Select fourth file mark' },
    },
  },
  {
    'mikavilpas/yazi.nvim',
    lazy = true,
    dependencies = { 'folke/snacks.nvim' },
    keys = {
      { '<leader>o', '<cmd>Yazi cwd<cr>', desc = 'Open Yazi at cwd' },
      { '<leader>O', '<cmd>Yazi<cr>', mode = { 'n', 'v' }, desc = 'Open Yazi' },
      { '<C-o>', '<cmd>Yazi toggle<cr>', desc = 'Resume Yazi' },
    },
    opts = {
      open_for_directories = false,
      keymaps = { show_help = '<F1>' },
    },
  },
  {
    'windwp/nvim-autopairs',
    event = 'InsertEnter',
    config = true,
  },
  {
    'windwp/nvim-ts-autotag',
    ft = { 'html', 'javascript', 'javascriptreact', 'typescript', 'typescriptreact' },
    opts = {
      opts = {
        enable_close = true,
        enable_rename = true,
        enable_close_on_slash = false,
      },
      per_filetype = {
        html = { enable_close = false },
      },
    },
  },
  {
    'lewis6991/gitsigns.nvim',
    event = { 'BufReadPre', 'BufNewFile' },
    opts = {},
    keys = {
      { '<leader>gh', function() require('gitsigns').preview_hunk() end, desc = 'Preview hunk' },
      { '<leader>gs', function() require('gitsigns').stage_hunk() end, mode = { 'n', 'v' }, desc = 'Stage hunk' },
      { '<leader>gr', function() require('gitsigns').reset_hunk() end, mode = { 'n', 'v' }, desc = 'Reset hunk' },
      { '<leader>gD', function() require('gitsigns').diffthis() end, desc = 'Diff file' },
      { '<leader>gL', function() require('gitsigns').blame_line() end, desc = 'Blame line' },
    },
  },
  {
    'hunk',
    virtual = true,
    keys = {
      { '<leader>hd', function() Snacks.terminal({ 'hunk', 'diff' }) end, desc = 'Review working tree' },
      { '<leader>hs', function() Snacks.terminal({ 'hunk', 'diff', '--staged' }) end, desc = 'Review staged changes' },
      { '<leader>hc', function() Snacks.terminal({ 'hunk', 'show', 'HEAD' }) end, desc = 'Review last commit' },
    },
  },
  {
    'nvim-tree/nvim-web-devicons',
    event = 'VeryLazy',
    opts = {},
  },
  {
    'nvim-lualine/lualine.nvim',
    event = 'VeryLazy',
    dependencies = { 'nvim-tree/nvim-web-devicons' },
    opts = {
      sections = { lualine_b = { 'grapple' } },
    },
  },
  {
    'HiPhish/rainbow-delimiters.nvim',
    event = { 'BufReadPre', 'BufNewFile' },
    config = function()
      require('rainbow-delimiters.setup').setup({})
    end,
  },
  {
    'NvChad/nvim-colorizer.lua',
    ft = { 'css', 'scss', 'less', 'html', 'javascript', 'javascriptreact', 'typescript', 'typescriptreact' },
    opts = {},
  },
  {
    'rachartier/tiny-inline-diagnostic.nvim',
    event = { 'BufReadPre', 'BufNewFile' },
    config = function()
      vim.diagnostic.config({ virtual_text = false })
      require('tiny-inline-diagnostic').setup()
    end,
  },
  {
    'mfussenegger/nvim-dap',
    ft = { 'javascript', 'javascriptreact', 'typescript', 'typescriptreact', 'go', 'rust' },
    dependencies = { 'igorlfs/nvim-dap-view' },
    keys = {
      { '<leader>Db', function() require('dap').toggle_breakpoint() end, desc = 'Toggle breakpoint' },
      { '<leader>Dc', function() require('dap').continue() end, desc = 'Continue / start' },
      { '<leader>Do', function() require('dap').step_over() end, desc = 'Step over' },
      { '<leader>Di', function() require('dap').step_into() end, desc = 'Step into' },
      { '<leader>DO', function() require('dap').step_out() end, desc = 'Step out' },
      { '<leader>Dr', function() require('dap').restart() end, desc = 'Restart' },
      { '<leader>Dx', function() require('dap').terminate() end, desc = 'Terminate' },
      { '<leader>Du', '<cmd>DapViewToggle<cr>', desc = 'Toggle debugger view' },
      { '<leader>De', function() require('dap').eval() end, mode = { 'n', 'x' }, desc = 'Evaluate expression' },
    },
    config = function()
      local dap = require('dap')

      local codelldb = vim.fn.exepath('codelldb')
      local js_debug = vim.fn.exepath('js-debug')

      dap.adapters.go = {
        type = 'server',
        port = '${port}',
        executable = {
          command = 'dlv',
          args = { 'dap', '-l', '127.0.0.1:${port}' },
        },
      }
      dap.configurations.go = {
        {
          type = 'go',
          name = 'Debug package',
          request = 'launch',
          program = '${workspaceFolder}',
        },
      }

      if codelldb ~= '' then
        dap.adapters.codelldb = {
          type = 'executable',
          command = codelldb,
        }
        dap.configurations.rust = {
          {
            type = 'codelldb',
            name = 'Launch executable',
            request = 'launch',
            program = function()
              return vim.fn.input('Executable: ', vim.fn.getcwd() .. '/', 'file')
            end,
            cwd = '${workspaceFolder}',
            stopOnEntry = false,
          },
        }
      end

      if js_debug ~= '' then
        dap.adapters['pwa-node'] = {
          type = 'server',
          host = '127.0.0.1',
          port = '${port}',
          executable = {
            command = js_debug,
            args = { '${port}' },
          },
        }
        for _, ft in ipairs({ 'javascript', 'javascriptreact', 'typescript', 'typescriptreact' }) do
          dap.configurations[ft] = {
            {
              type = 'pwa-node',
              request = 'launch',
              name = 'Launch current file',
              program = '${file}',
              cwd = '${workspaceFolder}',
              sourceMaps = true,
            },
          }
        end
      end
    end,
  },
  {
    'igorlfs/nvim-dap-view',
    opts = {
      auto_toggle = true,
      virtual_text = { enabled = true, position = 'inline' },
    },
  },
  {
    'neovim/nvim-lspconfig',
    ft = {
      'lua',
      'typescript', 'javascript', 'typescriptreact', 'javascriptreact',
      'html', 'css', 'scss', 'less',
      'bash', 'sh', 'fish', 'just', 'go', 'crystal', 'json', 'jsonc', 'yaml',
      'yaml.github', 'yaml.docker-compose', 'dockerfile', 'sql', 'systemd', 'toml',
    },
    config = function()
      require('lsp')
    end,
  },
  {
    'mrcjkb/rustaceanvim',
    ft = 'rust',
  },
  {
    'nvim-treesitter/nvim-treesitter',
    lazy = false,
    build = ':TSUpdate',
    config = function()
      local parsers = {
        'lua', 'vim', 'vimdoc', 'sql', 'bash', 'css', 'csv', 'gitignore',
        'go', 'gomod', 'gowork', 'gosum', 'html', 'jsdoc', 'json', 'rust',
        'markdown_inline', 'regex', 'scss', 'typescript', 'tsx', 'javascript',
        'just', 'yaml', 'toml',
      }

      require('nvim-treesitter').setup()
      require('nvim-treesitter').install(parsers)

      vim.api.nvim_create_autocmd('FileType', {
        pattern = {
          'lua', 'vim', 'vimdoc', 'sql', 'bash', 'sh', 'css', 'csv', 'gitignore',
          'go', 'gomod', 'gowork', 'gosum', 'html', 'javascript', 'javascriptreact',
          'json', 'markdown', 'rust', 'scss', 'typescript', 'typescriptreact', 'just', 'yaml', 'toml',
        },
        callback = function(args)
          pcall(vim.treesitter.start, args.buf)
          vim.bo[args.buf].indentexpr = "v:lua.require'nvim-treesitter'.indentexpr()"
        end,
      })
    end,
  },
  {
    'saghen/blink.cmp',
    event = { 'LspAttach', 'CmdlineEnter' },
    dependencies = {
      'saghen/blink.lib',
      'rafamadriz/friendly-snippets',
    },
    build = function()
      require('blink.cmp').build():pwait()
    end,
    ---@module 'blink.cmp'
    ---@type blink.cmp.Config
    opts = {
      keymap = { preset = 'default' },
      sources = {
        default = { 'lazydev', 'lsp', 'path', 'snippets', 'buffer' },
        providers = {
          lazydev = {
            name = 'LazyDev',
            module = 'lazydev.integrations.blink',
            score_offset = 100,
          },
        },
      },
    },
  },

}, {
  checker = { enabled = false },
  change_detection = { enabled = false },
  -- Home Manager owns ~/.config/nvim read-only; keep lazy.nvim's lockfile writable.
  lockfile = vim.fn.stdpath('state') .. '/lazy-lock.json',
})

require('dojo').setup()
