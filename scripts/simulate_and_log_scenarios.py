#!/usr/bin/env python3
import json
import uuid
import time
from datetime import datetime

# Path to write the log file
LOG_FILE = "logs/vci_scenarios_simulation.log"

class SimulationLogger:
    def __init__(self, filename):
        self.filename = filename
        # Ensure the logs directory exists
        import os
        os.makedirs(os.path.dirname(filename), exist_ok=True)
        # Clear existing file
        with open(self.filename, "w", encoding="utf-8") as f:
            f.write(f"=== VOS-RS VCI SCENARIOS SIMULATION LOG ===\n")
            f.write(f"Timestamp: {datetime.now().isoformat()}\n")
            f.write(f"===========================================\n\n")

    def log(self, tag, message, payload=None):
        timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]
        log_line = f"[{timestamp}] [{tag}] {message}\n"
        print(log_line.strip())
        with open(self.filename, "a", encoding="utf-8") as f:
            f.write(log_line)
            if payload:
                formatted_payload = json.dumps(payload, indent=2, ensure_ascii=False)
                f.write(f"Payload:\n{formatted_payload}\n\n")
                print(f"Payload:\n{formatted_payload}\n")

logger = SimulationLogger(LOG_FILE)

def make_event(event_type, call_id, sequence, data):
    return {
        "event_id": str(uuid.uuid4()),
        "schema_version": "1.0",
        "call_id": call_id,
        "sequence": sequence,
        "occurred_at_ms": int(time.time() * 1000),
        "event_type": event_type,
        "data": data
    }

async def simulate_scenario_1_dial_extension_first():
    """
    Scenario 1: 先呼分机 (A-leg)，再呼被叫 (B-leg)，最后桥接
    """
    logger.log("SCENARIO", "=== STARTING SCENARIO 1: Dial Extension First, then Callee, and Bridge ===")
    
    call_id_a = "call_leg_a_ext_10001"
    call_id_b = "call_leg_b_callee_153"
    
    # 1. Controller initiates Leg A (Extension)
    originate_a_cmd = {
        "call_id": call_id_a,
        "action": "originate",
        "target_uri": "sip:10001@127.0.0.1:5060",
        "caller_id": "Platform"
    }
    logger.log("COMMAND", "Controller sends Originate to Extension (Leg A)", originate_a_cmd)
    
    # B2BUA generates originated event for Leg A
    ev_initiated_a = make_event("call_originated", call_id_a, 1, {
        "target_uri": originate_a_cmd["target_uri"],
        "caller_id": originate_a_cmd["caller_id"],
        "leg": "b_leg"  # For originate, B2BUA is calling out to extension
    })
    logger.log("EVENT", "sip-edge reports Leg A originated", ev_initiated_a)
    
    # Ringing
    ev_ringing_a = make_event("call_ringing", call_id_a, 2, {
        "sip_status": 180,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg A is ringing", ev_ringing_a)
    
    # Answered
    ev_answered_a = make_event("call_answered", call_id_a, 3, {
        "sip_status": 200,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg A has answered", ev_answered_a)
    
    # 2. Leg A answered. Controller initiates Leg B (Callee)
    logger.log("LOGIC", f"Leg A ({call_id_a}) answered. Preparing to originate Leg B ({call_id_b}).")
    originate_b_cmd = {
        "call_id": call_id_b,
        "action": "originate",
        "target_uri": "sip:15300002222@gateway_ip:5060",
        "caller_id": "10001"
    }
    logger.log("COMMAND", "Controller sends Originate to Callee (Leg B)", originate_b_cmd)
    
    # B2BUA generates originated event for Leg B
    ev_initiated_b = make_event("call_originated", call_id_b, 1, {
        "target_uri": originate_b_cmd["target_uri"],
        "caller_id": originate_b_cmd["caller_id"],
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg B originated", ev_initiated_b)
    
    # Ringing
    ev_ringing_b = make_event("call_ringing", call_id_b, 2, {
        "sip_status": 183,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg B is ringing (Early Media)", ev_ringing_b)
    
    # Answered
    ev_answered_b = make_event("call_answered", call_id_b, 3, {
        "sip_status": 200,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg B has answered", ev_answered_b)
    
    # 3. Both answered. Bridge them!
    logger.log("LOGIC", "Both Leg A and Leg B answered. Sending bridge command.")
    bridge_cmd = {
        "call_id": "call_bridge_action_1",
        "action": "bridge",
        "call_id_a": call_id_a,
        "call_id_b": call_id_b
    }
    logger.log("COMMAND", "Controller sends Bridge command", bridge_cmd)
    
    # B2BUA generates bridged event
    ev_bridged = make_event("call_bridged", "call_bridge_action_1", 4, {
        "call_id_a": call_id_a,
        "call_id_b": call_id_b
    })
    logger.log("EVENT", "sip-edge reports successful bridge", ev_bridged)
    
    # Hangup by extension
    ev_finished_a = make_event("call_finished", call_id_a, 4, {
        "duration_secs": 15,
        "sip_status": 200,
        "q850_cause": 16,
        "reason": "Normal clearing",
        "leg": "a_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg A call finished", ev_finished_a)
    
    logger.log("SCENARIO", "=== SCENARIO 1 COMPLETED ===\n")

async def simulate_scenario_2_dial_callee_first():
    """
    Scenario 2: 先呼被叫 (A-leg)，再呼分机 (B-leg)，最后桥接
    """
    logger.log("SCENARIO", "=== STARTING SCENARIO 2: Dial Callee First, then Extension, and Bridge ===")
    
    call_id_a = "call_leg_a_callee_153"
    call_id_b = "call_leg_b_ext_10001"
    
    # 1. Controller initiates Leg A (Callee)
    originate_a_cmd = {
        "call_id": call_id_a,
        "action": "originate",
        "target_uri": "sip:15300002222@gateway_ip:5060",
        "caller_id": "Platform"
    }
    logger.log("COMMAND", "Controller sends Originate to Callee (Leg A)", originate_a_cmd)
    
    # Originated
    ev_initiated_a = make_event("call_originated", call_id_a, 1, {
        "target_uri": originate_a_cmd["target_uri"],
        "caller_id": originate_a_cmd["caller_id"],
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg A originated", ev_initiated_a)
    
    # Ringing
    ev_ringing_a = make_event("call_ringing", call_id_a, 2, {
        "sip_status": 180,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg A is ringing", ev_ringing_a)
    
    # Answered
    ev_answered_a = make_event("call_answered", call_id_a, 3, {
        "sip_status": 200,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg A has answered", ev_answered_a)
    
    # 2. Leg A answered. Controller initiates Leg B (Extension)
    logger.log("LOGIC", f"Leg A ({call_id_a}) answered. Preparing to originate Leg B ({call_id_b}).")
    originate_b_cmd = {
        "call_id": call_id_b,
        "action": "originate",
        "target_uri": "sip:10001@127.0.0.1:5060",
        "caller_id": "15300002222"
    }
    logger.log("COMMAND", "Controller sends Originate to Extension (Leg B)", originate_b_cmd)
    
    # Originated
    ev_initiated_b = make_event("call_originated", call_id_b, 1, {
        "target_uri": originate_b_cmd["target_uri"],
        "caller_id": originate_b_cmd["caller_id"],
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg B originated", ev_initiated_b)
    
    # Ringing
    ev_ringing_b = make_event("call_ringing", call_id_b, 2, {
        "sip_status": 180,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg B is ringing", ev_ringing_b)
    
    # Answered
    ev_answered_b = make_event("call_answered", call_id_b, 3, {
        "sip_status": 200,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg B has answered", ev_answered_b)
    
    # 3. Both answered. Bridge them!
    logger.log("LOGIC", "Both Leg A and Leg B answered. Sending bridge command.")
    bridge_cmd = {
        "call_id": "call_bridge_action_2",
        "action": "bridge",
        "call_id_a": call_id_a,
        "call_id_b": call_id_b
    }
    logger.log("COMMAND", "Controller sends Bridge command", bridge_cmd)
    
    # Bridged
    ev_bridged = make_event("call_bridged", "call_bridge_action_2", 4, {
        "call_id_a": call_id_a,
        "call_id_b": call_id_b
    })
    logger.log("EVENT", "sip-edge reports successful bridge", ev_bridged)
    
    logger.log("SCENARIO", "=== SCENARIO 2 COMPLETED ===\n")

async def simulate_scenario_3_conference():
    """
    Scenario 3: 会议室 (Conference Room Joining)
    """
    logger.log("SCENARIO", "=== STARTING SCENARIO 3: Conference Room Joining ===")
    
    call_id_a = "call_leg_a_ext_10001"
    call_id_b = "call_leg_b_callee_153"
    room_id = "room_8888"
    
    # 1. Originate Leg A
    originate_a_cmd = {
        "call_id": call_id_a,
        "action": "originate",
        "target_uri": "sip:10001@127.0.0.1:5060",
        "caller_id": "Platform"
    }
    logger.log("COMMAND", "Controller sends Originate to Extension (Leg A)", originate_a_cmd)
    
    ev_answered_a = make_event("call_answered", call_id_a, 1, {
        "sip_status": 200,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg A answered", ev_answered_a)
    
    # Push Leg A to Conference
    conf_a_cmd = {
        "call_id": call_id_a,
        "action": "conference",
        "room_id": room_id,
        "start_muted": False
    }
    logger.log("COMMAND", "Controller joins Leg A to conference room", conf_a_cmd)
    
    # 2. Originate Leg B
    originate_b_cmd = {
        "call_id": call_id_b,
        "action": "originate",
        "target_uri": "sip:15300002222@gateway_ip:5060",
        "caller_id": "Platform"
    }
    logger.log("COMMAND", "Controller sends Originate to Callee (Leg B)", originate_b_cmd)
    
    ev_answered_b = make_event("call_answered", call_id_b, 1, {
        "sip_status": 200,
        "leg": "b_leg"
    })
    logger.log("EVENT", "sip-edge reports Leg B answered", ev_answered_b)
    
    # Push Leg B to Conference
    conf_b_cmd = {
        "call_id": call_id_b,
        "action": "conference",
        "room_id": room_id,
        "start_muted": False
    }
    logger.log("COMMAND", "Controller joins Leg B to conference room", conf_b_cmd)
    
    logger.log("LOGIC", f"Both participants successfully joined conference room {room_id}.")
    logger.log("SCENARIO", "=== SCENARIO 3 COMPLETED ===\n")

async def main():
    await simulate_scenario_1_dial_extension_first()
    await simulate_scenario_2_dial_callee_first()
    await simulate_scenario_3_conference()
    logger.log("SYSTEM", f"All simulations finished successfully. Detailed log written to: {LOG_FILE}")

if __name__ == "__main__":
    import asyncio
    asyncio.run(main())
