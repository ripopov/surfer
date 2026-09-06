// Source-tile feature coverage: package types, a function, an interface with
// modports, generate branches selected per instance, a case statement and an
// include file with command-line defines.
`include "features_defs.svh"

package feat_pkg;
    typedef enum logic [1:0] {IDLE = 2'd0, RUN = 2'd1, DONE = 2'd2} state_t;
    typedef struct packed {
        logic       valid;
        logic [3:0] tag;
    } pkt_t;
    function automatic logic [3:0] inc_tag(input logic [3:0] tag);
        return tag + 4'd1;
    endfunction
endpackage

interface bus_if(input logic clk);
    logic       valid;
    logic [7:0] data;
    modport src(output valid, data, input clk);
    modport dst(input valid, data, clk);
endinterface

module counter #(parameter int W = `FEAT_WIDTH, parameter bit WRAP = 1'b1) (
    input  logic         clk,
    input  logic         rst_n,
    input  logic         en,
    output logic [W-1:0] count
);
    localparam logic [W-1:0] LIMIT = `FEAT_SAT_LIMIT;
    generate
        if (WRAP) begin : g_wrap
            always_ff @(posedge clk or negedge rst_n)
                if (!rst_n) count <= '0;
                else if (en) count <= count + 1'b1;
        end else begin : g_saturate
            always_ff @(posedge clk or negedge rst_n)
                if (!rst_n) count <= '0;
                else if (en && count != LIMIT) count <= count + 1'b1;
        end
    endgenerate
endmodule

module sink(bus_if.dst bus, output logic [7:0] seen);
    always_ff @(posedge bus.clk)
        if (bus.valid) seen <= bus.data;
endmodule

module top(
    input  logic       clk,
    input  logic       rst_n,
    input  logic       en,
    input  logic [1:0] mode,
    output logic [7:0] out
);
    import feat_pkg::*;
    localparam int LANES = 2;
    state_t     state;
    pkt_t       pkt;
    logic [7:0] lane_count[LANES];
    logic [7:0] seen;
    bus_if      bus(.clk(clk));

    for (genvar i = 0; i < LANES; i++) begin : g_lane
        counter #(.W(8), .WRAP(i == 0)) u(.clk(clk), .rst_n(rst_n), .en(en), .count(lane_count[i]));
    end
    sink u_sink(.bus(bus.dst), .seen(seen));

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) state <= IDLE;
        else begin
            case (mode)
                2'd0: state <= IDLE;
                2'd1: state <= RUN;
                default: state <= DONE;
            endcase
        end
    end

    always_comb begin
        pkt.valid = (state == RUN);
        pkt.tag   = inc_tag(lane_count[0][3:0]);
        bus.valid = pkt.valid;
        bus.data  = lane_count[1];
    end
    assign out = mode[0] ? lane_count[0] : seen;
endmodule
