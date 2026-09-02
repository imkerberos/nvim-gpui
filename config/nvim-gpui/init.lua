-- Repository-local Neovim configuration for `nvim-gpui` development.
--
-- It is loaded through XDG_CONFIG_HOME and NVIM_APPNAME from the development
-- flake and does not affect the user's normal Neovim profile.
vim.opt.termguicolors = true
vim.opt.number = true
vim.opt.title = true
vim.opt.guifont = "Iosevka Term Slab:h16"
vim.opt.guifontwide = "LXGW WenKai:h16"
vim.g.nvim_gpui = vim.env.NVIM_GPUI == "1"

-- The flake supplies all three plugins from the same nixpkgs revision. Lazy
-- still owns plugin startup, but its local specs never download from GitHub.
local lazy_dir = vim.env.NVIM_GPUI_LAZY
local snacks_dir = vim.env.NVIM_GPUI_SNACKS
local treesitter_dir = vim.env.NVIM_GPUI_TREESITTER

if lazy_dir and snacks_dir and treesitter_dir then
  vim.opt.rtp:prepend(lazy_dir)
  require("lazy").setup({
    {
      "nvim-treesitter/nvim-treesitter",
      dir = treesitter_dir,
      lazy = false,
      build = false,
    },
    {
      "folke/snacks.nvim",
      dir = snacks_dir,
      lazy = false,
      priority = 1000,
      opts = {
        image = {
          enabled = true,
          doc = { enabled = true },
        },
      },
    },
  }, {
    change_detection = { enabled = false },
    install = { missing = false },
    lockfile = vim.fn.stdpath("state") .. "/lazy-lock.json",
  })
  local terminal = require("snacks.image.terminal")
  local terminal_size = terminal.size()
  local function is_positive_finite(value)
    return type(value) == "number" and value > 0 and value < math.huge
  end

  -- An embedded Neovim has RPC pipes instead of a real terminal fd, so
  -- Snacks' ioctl(TIOCGWINSZ) probe can return zero pixel dimensions. Keep
  -- image cell calculations finite; Rust uses its own actual GPUI metrics for
  -- the final overlay placement.
  if not is_positive_finite(terminal_size.cell_width)
      or not is_positive_finite(terminal_size.cell_height) then
    local cell_width = tonumber(vim.env.NVIM_GPUI_CELL_WIDTH) or 9
    local cell_height = tonumber(vim.env.NVIM_GPUI_CELL_HEIGHT) or 18
    terminal_size = {
      width = vim.o.columns * cell_width,
      height = vim.o.lines * cell_height,
      columns = vim.o.columns,
      rows = vim.o.lines,
      cell_width = cell_width,
      cell_height = cell_height,
      scale = math.max(1, cell_width / 8),
    }
    terminal.size = function()
      return terminal_size
    end
  end
end
