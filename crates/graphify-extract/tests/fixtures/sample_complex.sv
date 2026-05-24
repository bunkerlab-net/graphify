// SystemVerilog with module, function, task, package import, instantiation.
package mypkg;
    typedef logic [7:0] byte_t;
endpackage

module alu #(parameter WIDTH = 8) (
    input  logic clk,
    input  logic rst,
    input  logic [WIDTH-1:0] a,
    input  logic [WIDTH-1:0] b,
    output logic [WIDTH-1:0] result
);
    import mypkg::*;

    function automatic logic [WIDTH-1:0] add_one(logic [WIDTH-1:0] x);
        return x + 1;
    endfunction

    task automatic reset_state();
        result = '0;
    endtask

    always_ff @(posedge clk or posedge rst) begin
        if (rst)
            result <= '0;
        else
            result <= a + b;
    end
endmodule

module top(input wire clk);
    logic [7:0] out;
    alu #(.WIDTH(8)) u_alu(.clk(clk), .rst(1'b0), .a(8'h01), .b(8'h02), .result(out));
endmodule
