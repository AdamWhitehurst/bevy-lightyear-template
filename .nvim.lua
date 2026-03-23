local function load_layout()
	vim.cmd("tabnew")
	vim.cmd("terminal")
	vim.api.nvim_buf_set_name(0, "server")
	vim.cmd("vsplit | terminal")
	vim.api.nvim_buf_set_name(0, "client")
	vim.cmd("split | terminal")
	vim.api.nvim_buf_set_name(0, "logs")
end

vim.keymap.set("n", "<leader>tl", load_layout)
