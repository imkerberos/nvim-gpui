-- Repository-local Neovim configuration for `nvim-gpui` development.
--
-- It is loaded through XDG_CONFIG_HOME and NVIM_APPNAME from the development
-- flake and does not affect the user's normal Neovim profile.
vim.opt.termguicolors = true
vim.opt.number = true
vim.opt.title = true
-- vim.opt.guifont = "Iosevka Term Slab:h16"
-- vim.opt.guifontwide = "LXGW WenKai:h16"
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

  local function setup_image_terminal_fallback()
    if vim.g.nvim_gpui ~= true then return end

    local cell_width = tonumber(vim.env.NVIM_GPUI_CELL_WIDTH) or 9
    local cell_height = tonumber(vim.env.NVIM_GPUI_CELL_HEIGHT) or 18
    local image_size
    local snacks_size
    local needs_resize_hook = false

    local function is_positive_finite(value)
      return type(value) == "number" and value > 0 and value < math.huge
    end

    local function has_cell_metrics(size)
      return size and is_positive_finite(size.cell_width)
        and is_positive_finite(size.cell_height)
    end

    local function update_size()
      local columns = vim.o.columns
      local rows = vim.o.lines
      image_size = {
        screen_x = columns * cell_width,
        screen_y = rows * cell_height,
        screen_cols = columns,
        screen_rows = rows,
        cell_width = cell_width,
        cell_height = cell_height,
      }
      snacks_size = {
        width = columns * cell_width,
        height = rows * cell_height,
        columns = columns,
        rows = rows,
        cell_width = cell_width,
        cell_height = cell_height,
        scale = math.max(1, cell_width / 8),
      }
    end

    local image_ok, image_terminal = pcall(require, "image/utils/term")
    if image_ok and type(image_terminal.get_size) == "function" then
      local size_ok, native_size = pcall(image_terminal.get_size)
      if not size_ok or not has_cell_metrics(native_size) then
        needs_resize_hook = true
        image_terminal.get_size = function() return image_size end
      end
    end

    local snacks_ok, snacks_terminal = pcall(require, "snacks.image.terminal")
    if snacks_ok and type(snacks_terminal.size) == "function" then
      local size_ok, native_size = pcall(snacks_terminal.size)
      if not size_ok or not has_cell_metrics(native_size) then
        needs_resize_hook = true
        snacks_terminal.size = function() return snacks_size end
      end
    end

    if needs_resize_hook then
      update_size()
      vim.api.nvim_create_autocmd("VimResized", { callback = update_size })
    end
  end

  setup_image_terminal_fallback()
end
