module source_tile_demo(input logic clk, input logic rst_n);
  logic [7:0] count;

  // A source tile keeps the selected declaration visible while debugging.
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n)
      count <= 8'h00;
    else
      count <= count + 8'd1;
  end
endmodule
