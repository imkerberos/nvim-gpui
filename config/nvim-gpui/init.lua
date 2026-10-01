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

-- The flake supplies lazy.nvim itself. Plugins supplied by Nix are loaded
-- directly from the store; if one is not present, lazy.nvim installs it into
-- the writable project-local data directory instead of trying to modify the
-- read-only Nix store.
local lazy_dir = vim.env.NVIM_GPUI_LAZY
assert(
  lazy_dir and vim.fn.isdirectory(lazy_dir) == 1,
  "NVIM_GPUI_LAZY is missing; run nvim-gpui inside nix develop"
)

vim.opt.rtp:prepend(lazy_dir)

local function nix_or_lazy_spec(repo, env_name, opts, fallback_opts)
  local spec = vim.tbl_extend("force", { repo }, opts or {})
  local nix_dir = vim.env[env_name]

  if nix_dir and vim.fn.isdirectory(nix_dir) == 1 then
    spec.dir = nix_dir
  elseif fallback_opts then
    spec = vim.tbl_extend("force", spec, fallback_opts)
  end

  return spec
end

require("lazy").setup({
  nix_or_lazy_spec(
    "nvim-treesitter/nvim-treesitter",
    "NVIM_GPUI_TREESITTER",
    { lazy = false, build = false },
    { build = ":TSUpdate" }
  ),
  nix_or_lazy_spec("folke/snacks.nvim", "NVIM_GPUI_SNACKS", {
    lazy = false,
    priority = 1000,
    opts = {
      image = {
        enabled = true,
        doc = { enabled = true },
      },
    },
  }),
  nix_or_lazy_spec("OXY2DEV/markview.nvim", "NVIM_GPUI_MARKVIEW", {
    lazy = false,
    opts = {
      preview = {
        enable_hybrid_mode = true,
        hybrid_modes = { "n" },
        modes = { "n", "no", "c" },
      },
    },
  }),
}, {
  root = vim.fn.stdpath("data") .. "/lazy",
  change_detection = { enabled = false },
  install = { missing = true },
  lockfile = vim.fn.stdpath("state") .. "/lazy-lock.json",
})

-- Configure both generations of nvim-treesitter. The current Nix package uses
-- the newer API, which requires starting a highlighter for each filetype.
local has_configs_module = pcall(require, "nvim-treesitter.configs")
if has_configs_module then
  require("nvim-treesitter.configs").setup({
    highlight = { enable = true },
    indent = { enable = true },
    matchup = { enable = false },
  })
else
  local treesitter_ok, treesitter = pcall(require, "nvim-treesitter")
  if treesitter_ok and type(treesitter.setup) == "function" then
    treesitter.setup({
      highlight = { enable = true },
      indent = { enable = true },
      matchup = { enable = false },
    })

    local treesitter_group = vim.api.nvim_create_augroup(
      "nvim-gpui-treesitter",
      { clear = true }
    )
    vim.api.nvim_create_autocmd("FileType", {
      group = treesitter_group,
      pattern = "*",
      callback = function(args)
        local filetype = vim.bo[args.buf].filetype
        local language = vim.treesitter.language.get_lang(filetype) or filetype
        pcall(vim.treesitter.start, args.buf, language)
        vim.bo[args.buf].indentexpr = "v:lua.require'nvim-treesitter'.indentexpr()"
      end,
    })
  end
end

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
